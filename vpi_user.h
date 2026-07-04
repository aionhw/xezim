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

/* Flags for vpi_put_value */
#define vpiNoDelay              0
#define vpiInertialDelay        1
#define vpiTransportDelay       2
#define vpiPureTransportDelay   3
#define vpiForceFlag            4
#define vpiReleaseFlag          5

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

/* s_vpi_vecval - vector value representation */
typedef struct t_vpi_vecval {
    VPI_UINT32 aval;
    VPI_UINT32 bval;
} s_vpi_vecval, *p_vpi_vecval;

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
vpiHandle vpi_handle(vpiHandle object, int type);
int vpi_get(int property, vpiHandle object);
void vpi_get_value(vpiHandle object, p_vpi_value value_p);
vpiHandle vpi_put_value(vpiHandle object, p_vpi_value value_p, p_vpi_time time_p, int flags);
int vpi_free_object(vpiHandle object);

#endif /* VPI_USER_H */
