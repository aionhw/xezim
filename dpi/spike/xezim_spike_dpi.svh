// SV-side DPI-C imports for the xezim Spike shim.
//
// Load the matching .so at the xezim command line:
//     xezim ... --dpi-lib ./xezim_spike_dpi.so ...
// then `import "DPI-C"` the symbols below from any SV scope that needs
// to drive Spike (typically a UVM ISS wrapper or `step_compare`).

`ifndef __XEZIM_SPIKE_DPI_SVH__
`define __XEZIM_SPIKE_DPI_SVH__

// --- lifecycle --------------------------------------------------------

// Initialize Spike with an ELF program. ISA / ABI strings follow the
// Spike convention ("rv32imc" / "ilp32" for cv32e40p-class).
// Returns 0 on success, non-zero on error.
import "DPI-C" function int xezim_spike_init(input string elf_path,
                                             input string isa,
                                             input string priv);

// Step one instruction. Outputs are valid iff the return value is 1.
//   retired_pc   : address of the just-retired instruction
//   retired_insn : raw encoded instruction word (32 bits for RV32)
//   rd           : destination register index (0..31), or -1 if none
//   rd_val       : destination register value after the write
import "DPI-C" function int xezim_spike_step(output longint unsigned retired_pc,
                                             output int unsigned     retired_insn,
                                             output int              rd,
                                             output longint unsigned rd_val);

// Read a GPR or the current PC (idx 32 == PC).
import "DPI-C" function longint unsigned xezim_spike_get_reg(input int idx);
import "DPI-C" function longint unsigned xezim_spike_get_pc();

// Optional finish — release Spike resources.
import "DPI-C" function void xezim_spike_finish();

`endif // __XEZIM_SPIKE_DPI_SVH__
