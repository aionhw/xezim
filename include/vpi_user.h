#ifndef VPI_USER_H
#define VPI_USER_H

#include <stdint.h>
#include <stddef.h>

/* Basic VPI types */
typedef uint32_t VPI_UINT32;
typedef int32_t VPI_INT32;
typedef uint64_t VPI_UINT64;
typedef int64_t VPI_INT64;

/* Object types */
#define vpiModule 1
#define vpiPort 2
#define vpiWire 3
#define vpiReg 4
#define vpiIntegerVar 5
#define vpiRealVar 6
#define vpiTimeVar 7
#define vpiShortRealVar 8
#define vpiMemoryWord 9
#define vpiMemory 10
#define vpiParameter 11
#define vpiStructVar 12
#define vpiUnionVar 13
#define vpiClassVar 14
#define vpiEnumVar 15
#define vpiPackedArrayVar 16
#define vpiArray 17
#define vpiNet 18
#define vpiBitVar 19

/* vpiNetBit / vpiRegBit / vpiPartSelect / vpiBitSelect are the
 * indexed-element variants used by UVM's HDL polling to descend
 * into packed arrays and bit-selects of a parent vpiReg/vpiNet. */
#ifndef vpiNetBit
#define vpiNetBit       20
#endif
#ifndef vpiRegBit
#define vpiRegBit       21
#endif
#ifndef vpiPartSelect
#define vpiPartSelect   22
#endif
#ifndef vpiBitSelect
#define vpiBitSelect    23
#endif

/* vpiIntVar / vpiLongIntVar / vpiShortIntVar / vpiByteVar — the
 * `vpi_get(vpiType, ...)` codes returned for the corresponding
 * SystemVerilog variable types. Used by UVM's HDL polling to choose
 * its read-modify-write word size. */
#ifndef vpiIntVar
#define vpiIntVar       24
#endif
#ifndef vpiLongIntVar
#define vpiLongIntVar   25
#endif
#ifndef vpiShortIntVar
#define vpiShortIntVar  26
#endif
#ifndef vpiByteVar
#define vpiByteVar      27
#endif
#ifndef vpiArrayVar
#define vpiArrayVar     28
#endif
#ifndef vpiArrayNet
#define vpiArrayNet     29
#endif
#ifndef vpiClassVar
#define vpiClassVar     30
#endif

/* Format codes for vpi_get_value / vpi_put_value */
#define vpiBinStrVal    1
#define vpiOctStrVal    2
#define vpiDecStrVal    3
#define vpiHexStrVal    4
#define vpiStringVal    5
#define vpiRealVal      6
#define vpiVectorVal    7
#define vpiNullVal      8
#define vpiScalarVal    9
#define vpiIntVal       10
#define vpiLogicVal     11
#define vpiObjTypeVal   12
#define vpiVector4Val   13
#define vpiSmallIntVal  14
#define vpiLongIntVal   15
#define vpiShortIntVal  16
#define vpiByteVal      17
#define vpiWordVal      18
#define vpiShortWordVal 19

/* Flags for vpi_put_value (IEEE 1800-2017 Table 38-44). */
#define vpiNoDelay              1
#define vpiInertialDelay        2
#define vpiTransportDelay       3
#define vpiPureTransportDelay   4
#define vpiForceFlag            5
#define vpiReleaseFlag          6
#define vpiCancelForce          7

/* vpi_get codes */
#define vpiType               1
#define vpiName               2
#define vpiFullName           3
#define vpiSize               4
#define vpiSigned             5
#define vpiLeftRange          6
#define vpiRightRange         7
#define vpiHighConn           8
#define vpiLowConn            9
#define vpiScalar             10
#define vpiVector             11
#define vpiTableEntry         13

/* VPI time types */
#define vpiSimTime            0
#define vpiScaledRealTime     1
#define vpiSuppressTime       2

/* s_vpi_vecval - vector value representation. Layout-compatible
 * with svLogicVecVal (IEEE 1800 §35.5.5) so UVM's HDL backdoor can
 * memcpy between the two without per-field translation. */
typedef struct t_vpi_vecval {
    VPI_UINT32 aval;
    VPI_UINT32 bval;
} s_vpi_vecval, *p_vpi_vecval;

/* vpiSuppressVal — "don't actually write; just compute". UVM's
 * vpi_get_value call uses this to fetch the current 4-state vector
 * without forcing a recompute. */
#ifndef vpiSuppressVal
#define vpiSuppressVal  1
#endif

/* vpi0 / vpi1 — 4-state scalar values used in vpi_get_value with
 * vpiScalarVal format (returned by vpi_get with vpiScalar property).
 * UVM's HDL polling toggle detector compares against these. */
#ifndef vpi0
#define vpi0  0
#endif
#ifndef vpi1
#define vpi1  1
#endif
#ifndef vpiX
#define vpiX  2
#endif
#ifndef vpiZ
#define vpiZ  3
#endif

/* s_vpi_value - value structure */
typedef struct t_vpi_value {
    int format;
    union {
        VPI_INT32 integer;
        double real;
        VPI_UINT64 time;
        char* str;
        struct t_vpi_vecval* vector;
        VPI_UINT32 scalar;
        VPI_INT64 longint;
    } value;
} s_vpi_value, *p_vpi_value;

/* s_vpi_time - time structure */
typedef struct t_vpi_time {
    int type;
    VPI_UINT32 high;
    VPI_UINT32 low;
    double real;
} s_vpi_time, *p_vpi_time;

/* vpiHandle - opaque handle to simulation objects */
typedef void* vpiHandle;

/* VPI function declarations */
vpiHandle vpi_handle_by_name(char* name, void* scope);
/* PLI typedefs — IEEE 1800 §38 uses these for portable integer widths.
 * xezim's Rust side stores them as plain `isize`/`i64`, so we mirror
 * them to the host's natural widths. */
#ifndef PLI_INT32
#define PLI_INT32 int
#endif
#ifndef PLI_INT64
#define PLI_INT64 long long
#endif
#ifndef PLI_UINT32
#define PLI_UINT32 unsigned int
#endif
#ifndef PLI_BYTE8
#define PLI_BYTE8 signed char
#endif

/* --- minimal s_vpi_vlog_info for vpi_get_vlog_info --- */
typedef struct t_vpi_vlog_info {
    int argc;
    char **argv;
    char *product;
    char *version;
} s_vpi_vlog_info, *p_vpi_vlog_info;

/* VPI callback reason codes (subset). */
#define cbValueChange        6
#define cbStartOfReset      15

/* s_cb_data - matches IEEE 1800 §38.7. Used by vpi_register_cb and
 * by the value-change dispatcher in simulator.rs. The cb_rtn
 * signature is `PLI_INT32 (*)(p_cb_data)` per the IEEE standard
 * (UVM's polling framework relies on the cb_data argument).
 * Forward-declare to allow the function pointer field to reference
 * p_cb_data without an ordering trap. */
typedef struct t_cb_data s_cb_data, *p_cb_data;
struct t_cb_data {
    int           reason;
    PLI_INT32   (*cb_rtn)(p_cb_data cb_data_p);
    vpiHandle     obj;
    p_vpi_time    time;
    p_vpi_value   value;
    void         *user_data;
};

vpiHandle vpi_handle(vpiHandle object, int type);
int vpi_get(int property, vpiHandle object);
void vpi_get_value(vpiHandle object, p_vpi_value value_p);
vpiHandle vpi_put_value(vpiHandle object, p_vpi_value value_p, p_vpi_time time_p, int flags);
int vpi_free_object(vpiHandle object);
void *vpi_register_cb(p_cb_data cb_data_p);
int vpi_remove_cb(void *cb);
int vpi_get_vlog_info(p_vpi_vlog_info info_p);

/* DPI scope/runtime primitives are declared in svdpi.h with their
 * proper `svScope` opaque-pointer type. vpi_user.h historically
 * forward-declared them as `void *` (a holdover from the Accellera
 * vpi_compatibility.h shim) but those older declarations conflict
 * with svdpi.h's typed ones when both are included. Including
 * svdpi.h here resolves the symbol with the correct type. */
#include "svdpi.h"

#endif /* VPI_USER_H */
