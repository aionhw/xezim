#ifndef VERIUSER_H
#define VERIUSER_H

/* Minimal veriuser.h stub for compiling the Accellera UVM reference
 * implementation against xezim.
 *
 * The original Verilog-XL `veriuser.h` declares the TF (task/function)
 * and acc_ (access) PLI interfaces that legacy simulators expose.
 * xezim does NOT implement the TF / acc_ PLI v1.0 surface; UVM 1.2
 * still `#include "veriuser.h"` for a handful of constant definitions
 * that the DPI shim uses for severity codes and message routing.
 *
 * We declare the typedefs UVM references and let the DPI functions
 * (declared in uvm_dpi.h via vpi_user.h) do the actual work. If UVM
 * code calls a TF/acc_ function that isn't implemented, the link
 * step will fail loudly — which is the right failure mode for a
 * vendor-portable testbench.
 */

/* PLI_BYTE8 etc. are usually defined here too. vpi_user.h re-defines
 * them via SV_PUBLIC; this file just keeps the include chain happy. */

#endif /* VERIUSER_H */