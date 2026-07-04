#ifndef SVDPI_H
#define SVDPI_H

/* Minimal SystemVerilog DPI (IEEE 1800 sections 35-36) header for xezim.
 *
 * xezim implements the subset of DPI that user code actually calls:
 * scalar/vector import arguments, open arrays for unpacked array
 * passing, and the four SV scope primitives. The full IEEE 1800
 * header is ~900 lines and includes things xezim does not support
 * (c bit-select returns by reference, packed array handles, etc.).
 */

#include <stdint.h>
#include <limits.h>

/* s_vpi_vecval is declared in vpi_user.h (IEEE 1800 section 38.25).
 * svdpi.h needs it for svLogicVecVal below. Pulling vpi_user.h in
 * here keeps the dependency explicit instead of relying on
 * ordering between user #include directives. */
#include "vpi_user.h"

/* svBitVecVal - unsigned 32-bit vector element used for svBit types. */
typedef uint32_t svBitVecVal;

/* svLogicVecVal - 4-state vector element. IEEE 1800 section 35.5.5
 * specifies the layout as identical to s_vpi_vecval (aval/bval);
 * UVM source files do plain assignment between the two, so we
 * typedef to keep type compatibility without per-field translation. */
typedef s_vpi_vecval svLogicVecVal;

typedef svLogicVecVal* p_svLogicVecVal;

/* Open array types */
typedef struct svOpenArrayType {
    void* dhandle;
    void* dptr;
    int dims[16];
    int static_size[16];
} svOpenArrayType;

typedef void* svOpenArrayHandle;

/* svScope - opaque scope handle. Pointer-only; never dereferenced
 * from C. Forward-declared so callers can store and pass it. */
typedef struct SVScopePlaceholder *svScope;

/* Symbol visibility for export functions. */
#define SV_PUBLIC __attribute__((visibility("default")))

/* DPI context function attribute. C and C++ see different syntax:
 *   C++ : extern "C" __attribute__((visibility("default")))
 *   C   : __attribute__((visibility("default"))) only
 * Both UVM `uvm_common.c` (C) and `uvm_dpi.cc` (C++) need this to
 * compile cleanly against the same header. */
#ifdef __cplusplus
#define DPI_CONTEXT extern "C" SV_PUBLIC
#else
#define DPI_CONTEXT SV_PUBLIC
#endif

/* DPI version query - IEEE 1800 section 35.7. Returns the DPI
 * standard revision as a string ("1800-2005", "1800-2009", etc.).
 * UVM's m_uvm_report_dpi calls this at startup to confirm the
 * simulator supports the DPI version UVM expects. */
DPI_CONTEXT const char *svDpiVersion(void);

/* Scope primitives - IEEE 1800 section 36.6. svSetScope returns the
 * previously-active scope so callers can save/restore the stack:
 *
 *     svScope prev = svSetScope(my_scope);
 *     ... do work ...
 *     svSetScope(prev);
 *
 * IEEE 1800-2005 declared `void svSetScope(svScope)` but 1800-2009+
 * added the svScope return. UVM 1.2 (which calls this pattern) was
 * written against the 1800-2009+ behavior. */
DPI_CONTEXT svScope svGetScopeFromName(const char *scope_name);
DPI_CONTEXT const char *svGetNameFromScope(svScope scope);
DPI_CONTEXT svScope svGetScope(void);
DPI_CONTEXT svScope svSetScope(svScope scope);

/* Compatibility marker for tools that test which DPI standard we
 * expose. The 1800-2005 value 0 is the UVM-required minimum. */
#ifndef DPI_COMPATIBILITY_VERSION_1800_2005
#define DPI_COMPATIBILITY_VERSION_1800_2005  0
#endif

#endif /* SVDPI_H */