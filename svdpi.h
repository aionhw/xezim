#ifndef SVDPI_H
#define SVDPI_H

#include <stdint.h>

/* svBitVecVal - used for passing vector values through DPI */
typedef uint32_t svBitVecVal;

/* Open array types */
typedef struct svOpenArrayType {
    void* dhandle;
    void* dptr;
    int dims[16];
    int static_size[16];
} svOpenArrayType;

typedef void* svOpenArrayHandle;

#define SV_PUBLIC __attribute__((visibility("default")))

/* DPI context function attribute */
#define DPI_CONTEXT extern "C" SV_PUBLIC

#endif /* SVDPI_H */
