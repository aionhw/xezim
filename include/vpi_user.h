#ifndef VPI_USER_H
#define VPI_USER_H

/* VPI (Verilog Procedural Interface) — IEEE 1800-2017 Annex K.
 *
 * Every constant below is the value the standard assigns it, so a C
 * file compiled against a vendor `vpi_user.h` and linked to xezim
 * agrees with xezim about what `vpiIntVal` or `cbValueChange` means.
 * An earlier version of this header invented its own numbering, which
 * meant any such file silently took the wrong branch.
 *
 * xezim implements the subset declared at the bottom of this file.
 * Functions the standard defines but xezim does not implement are NOT
 * declared here: a call to one is a compile error, which is the loud
 * failure we want, rather than a link-time surprise or a stub that
 * silently returns nothing. Notably absent: vpi_iterate / vpi_scan
 * (object traversal), vpi_get_str, vpi_printf, vpi_control,
 * vpi_chk_error, vpi_register_systf, and `vlog_startup_routines`.
 * There is no VPI module loading path — xezim's VPI is reachable only
 * from a DPI shared object loaded with `--dpi-lib`.
 */

#include <stdint.h>
#include <stddef.h>

/* PLI type definitions (IEEE 1800-2017 Annex K.1). */
typedef int32_t  PLI_INT32;
typedef uint32_t PLI_UINT32;
typedef int64_t  PLI_INT64;
typedef uint64_t PLI_UINT64;
typedef char     PLI_BYTE8;
typedef short    PLI_INT16;
typedef unsigned short PLI_UINT16;

/* vpiHandle — opaque handle to a simulation object. */
typedef PLI_UINT32 *vpiHandle;

/* --- vpi_get(vpiType, ...) object types (Annex K, "Object types") ------
 * Only the codes xezim can actually return are listed. `vpiLogicVar` is
 * an alias of `vpiReg`, exactly as in the standard header. */
#define vpiIntegerVar         25   /* integer variable */
#define vpiModule             32
#define vpiNet                36   /* scalar or vector net */
#define vpiParameter          41
#define vpiRealVar            47   /* real variable */
#define vpiReg                48   /* scalar or vector reg (4-state) */
#define vpiTimeVar            63
/* SystemVerilog object types (IEEE 1800-2017 sv_vpi_user.h). */
#define vpiLongIntVar        610
#define vpiShortIntVar       611
#define vpiIntVar            612
#define vpiShortRealVar      613
#define vpiByteVar           614
#define vpiStringVar         616
#define vpiEnumVar           617
#define vpiStructVar         618
#define vpiUnionVar          619
#define vpiBitVar            620   /* 2-state bit variable */
#define vpiLogicVar         vpiReg /* 4-state logic variable */

/* --- vpi_get_value / vpi_put_value format codes (Table 38-44) --------- */
#define vpiBinStrVal           1
#define vpiOctStrVal           2
#define vpiDecStrVal           3
#define vpiHexStrVal           4
#define vpiScalarVal           5
#define vpiIntVal              6
#define vpiRealVal             7
#define vpiStringVal           8
#define vpiVectorVal           9
#define vpiStrengthVal        10   /* not supported by xezim */
#define vpiTimeVal            11
#define vpiObjTypeVal         12
#define vpiSuppressVal        13

/* --- vpiScalarVal codes ----------------------------------------------- */
#define vpi0                   0
#define vpi1                   1
#define vpiZ                   2
#define vpiX                   3
#define vpiH                   4
#define vpiL                   5
#define vpiDontCare            6

/* --- vpi_put_value flags ---------------------------------------------- */
#define vpiNoDelay             1
#define vpiInertialDelay       2
#define vpiTransportDelay      3
#define vpiPureTransportDelay  4
#define vpiForceFlag           5
#define vpiReleaseFlag         6

/* --- vpi_get properties ----------------------------------------------- */
#define vpiUndefined         (-1)
#define vpiType                1
#define vpiName                2
#define vpiFullName            3
#define vpiSize                4
#define vpiScalar             17
#define vpiVector             18
#define vpiSigned             65

/* --- vpi_time types --------------------------------------------------- */
#define vpiScaledRealTime      1
#define vpiSimTime             2
#define vpiSuppressTime        3

/* --- callback reasons (Table 38-49) ----------------------------------- */
#define cbValueChange          1
#define cbStartOfReset        19
#define cbEndOfReset          20

/* s_vpi_vecval — 4-state vector element (IEEE 1800-2017 §38.10.1).
 * Layout-compatible with svLogicVecVal (§35.5.5), so UVM's HDL backdoor
 * can assign between the two without translation.
 *
 * Bit encoding, per element bit i:
 *     aval bval   value
 *       0    0      0
 *       1    0      1
 *       0    1      Z
 *       1    1      X
 */
typedef struct t_vpi_vecval {
    PLI_INT32 aval;
    PLI_INT32 bval;
} s_vpi_vecval, *p_vpi_vecval;

/* s_vpi_time — time value. */
typedef struct t_vpi_time {
    PLI_INT32 type;    /* vpiSimTime / vpiScaledRealTime / vpiSuppressTime */
    PLI_UINT32 high;
    PLI_UINT32 low;
    double real;
} s_vpi_time, *p_vpi_time;

/* s_vpi_value — value in one of the formats above. */
typedef struct t_vpi_value {
    PLI_INT32 format;
    union {
        PLI_BYTE8            *str;
        PLI_INT32             scalar;
        PLI_INT32             integer;
        double                real;
        struct t_vpi_time    *time;
        struct t_vpi_vecval  *vector;
        PLI_BYTE8            *misc;
    } value;
} s_vpi_value, *p_vpi_value;

/* s_vpi_vlog_info — tool identification, filled by vpi_get_vlog_info. */
typedef struct t_vpi_vlog_info {
    PLI_INT32   argc;
    PLI_BYTE8 **argv;
    PLI_BYTE8  *product;
    PLI_BYTE8  *version;
} s_vpi_vlog_info, *p_vpi_vlog_info;

/* s_cb_data — callback registration and dispatch (IEEE 1800-2017 §38.7). */
typedef struct t_cb_data s_cb_data, *p_cb_data;
struct t_cb_data {
    PLI_INT32    reason;
    PLI_INT32  (*cb_rtn)(p_cb_data cb_data_p);
    vpiHandle    obj;
    p_vpi_time   time;
    p_vpi_value  value;
    PLI_INT32    index;
    PLI_BYTE8   *user_data;
};

/* ---------------------------------------------------------------------
 * Implemented by xezim. Signatures match IEEE 1800-2017 Annex K exactly.
 * ------------------------------------------------------------------ */

/* Resolve a hierarchical name. `scope` is ignored (xezim resolves against
 * the flat signal table); pass NULL. Returns NULL if the name does not
 * name a signal. Tries the full name, then each successively shorter
 * suffix, so "top.dut.sig", "dut.sig" and "sig" all resolve. */
vpiHandle vpi_handle_by_name(PLI_BYTE8 *name, vpiHandle scope);

/* xezim models no object relationships, so this always returns NULL —
 * the standard's answer for "that relationship does not exist". It is
 * declared so that code which probes for a relationship links and takes
 * its NULL path rather than failing to load. */
vpiHandle vpi_handle(PLI_INT32 type, vpiHandle refHandle);

/* Returns vpiUndefined (-1) for a property xezim does not model.
 * Supported: vpiType, vpiSize, vpiSigned, vpiScalar, vpiVector. */
PLI_INT32 vpi_get(PLI_INT32 property, vpiHandle object);

/* On success, fills *value_p in the requested format. On failure — a bad
 * handle, or a format xezim cannot supply — sets value_p->format to
 * vpiSuppressVal and writes nothing else (IEEE 1800-2017 §38.16), which
 * is the ONLY way a caller can detect the failure. Always check it.
 *
 * For vpiVectorVal, vpiStringVal, the *StrVal formats and vpiTimeVal, the
 * returned pointer addresses simulator-owned storage that is valid only
 * until the next vpi_get_value call on this thread. Copy it out. */
void vpi_get_value(vpiHandle expr, p_vpi_value value_p);

/* Writes value_p to the object. flags selects vpiNoDelay (immediate),
 * vpiForceFlag or vpiReleaseFlag; the delay flags behave as vpiNoDelay
 * because xezim has no VPI event scheduling. Returns NULL. A format
 * xezim cannot decode writes nothing and warns. */
vpiHandle vpi_put_value(vpiHandle object, p_vpi_value value_p,
                        p_vpi_time time_p, PLI_INT32 flags);

PLI_INT32 vpi_free_object(vpiHandle object);
PLI_INT32 vpi_release_handle(vpiHandle object);
PLI_INT32 vpi_get_vlog_info(p_vpi_vlog_info vlog_info_p);

/* Only cbValueChange and cbStartOfReset are dispatched. Any other reason
 * is rejected with a NULL return rather than silently accepted. When a
 * cbValueChange fires, cb_data_p->obj, ->time and ->value are populated;
 * ->value uses the format of the value struct supplied at registration
 * (vpiIntVal if none was given). */
vpiHandle vpi_register_cb(p_cb_data cb_data_p);
PLI_INT32 vpi_remove_cb(vpiHandle cb_obj);

/* DPI scope/runtime primitives live in svdpi.h with their proper
 * `svScope` type. Included here so both are visible together. */
#include "svdpi.h"

#endif /* VPI_USER_H */
