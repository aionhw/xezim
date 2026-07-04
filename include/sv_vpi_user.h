#ifndef SV_VPI_USER_H
#define SV_VPI_USER_H

/* Compatibility shim for the Accellera UVM reference implementation.
 *
 * Some UVM source files (`uvm_hdl_polling.c`, the VCS backend)
 * `#include "sv_vpi_user.h"` directly. The full Accellera header
 * declares vpiTypes.h contents and the vlog_chk_error / io_printf
 * family of legacy PLI v1.0 helpers — xezim doesn't implement
 * those, but the C compile needs the include chain to resolve.
 *
 * The actual types UVM uses (`svScope`, `svLogicVecVal`) now live
 * in `svdpi.h` per IEEE 1800 §35.5.5. This file just re-includes
 * `svdpi.h` and provides a few extra typedefs the legacy header
 * is expected to define.
 */

#include "svdpi.h"
#include "vpi_user.h"

/* Legacy typedefs from the full sv_vpi_user.h that some UVM source
 * files still expect to be visible after `#include "sv_vpi_user.h"`.
 * The full Accellera header defines these via vpi_user.h + vpi_compatibility.h;
 * for xezim, both types are already available from vpi_user.h. */

#endif /* SV_VPI_USER_H */