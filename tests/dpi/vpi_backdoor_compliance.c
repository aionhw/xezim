#include <stdio.h>
#include <string.h>
#include <stdlib.h>
#include "vpi_user.h"
#include "svdpi.h"

// Helper to resolve handle
static vpiHandle get_handle(const char* path) {
    vpiHandle h = vpi_handle_by_name((char*)path, NULL);
    if (!h) {
        fprintf(stderr, "[VPI Backdoor] Failed to resolve path: %s\n", path);
    }
    return h;
}

// ----------------------------------------------------
// INT APIs (covers byte, shortint, int, longint)
// ----------------------------------------------------
int backdoor_read_int(const char* path, int* val) {
    vpiHandle h = get_handle(path);
    if (!h) return 0;
    s_vpi_value value_s;
    value_s.format = vpiIntVal;
    vpi_get_value(h, &value_s);
    // vpiSuppressVal is the ONLY failure channel vpi_get_value has
    // (IEEE 1800-2017 section 38.16); the function itself returns void.
    if (value_s.format != vpiIntVal) { vpi_free_object(h); return 0; }
    *val = value_s.value.integer;
    vpi_free_object(h);
    return 1;
}

int backdoor_force_int(const char* path, int val) {
    vpiHandle h = get_handle(path);
    if (!h) return 0;
    s_vpi_value value_s;
    value_s.format = vpiIntVal;
    value_s.value.integer = val;
    s_vpi_time time_s = {vpiSimTime, 0, 0, 0.0};
    vpi_put_value(h, &value_s, &time_s, vpiForceFlag);
    vpi_free_object(h);
    return 1;
}

// ----------------------------------------------------
// REAL APIs (covers real, shortreal)
// ----------------------------------------------------
int backdoor_read_real(const char* path, double* val) {
    vpiHandle h = get_handle(path);
    if (!h) return 0;
    s_vpi_value value_s;
    value_s.format = vpiRealVal;
    vpi_get_value(h, &value_s);
    if (value_s.format != vpiRealVal) { vpi_free_object(h); return 0; }
    *val = value_s.value.real;
    vpi_free_object(h);
    return 1;
}

int backdoor_force_real(const char* path, double val) {
    vpiHandle h = get_handle(path);
    if (!h) return 0;
    s_vpi_value value_s;
    value_s.format = vpiRealVal;
    value_s.value.real = val;
    s_vpi_time time_s = {vpiSimTime, 0, 0, 0.0};
    vpi_put_value(h, &value_s, &time_s, vpiForceFlag);
    vpi_free_object(h);
    return 1;
}

// ----------------------------------------------------
// VECTOR APIs (covers logic/bit vectors up to 128-bit)
// ----------------------------------------------------
// vpi_get_value does NOT fill a caller-supplied buffer for vpiVectorVal:
// it points value_s.value.vector at SIMULATOR-owned storage that is valid
// only until the next vpi_get_value call (IEEE 1800-2017 section 38.16).
// Copy out of it; never assume the caller's pointer was used.
int backdoor_read_vec128(const char* path, svBitVecVal* val) {
    vpiHandle h = get_handle(path);
    if (!h) return 0;
    s_vpi_value value_s;
    value_s.format = vpiVectorVal;
    vpi_get_value(h, &value_s);
    if (value_s.format != vpiVectorVal || value_s.value.vector == NULL) {
        vpi_free_object(h);
        return 0;
    }

    int size = vpi_get(vpiSize, h);
    int num_words = (size + 31) / 32;
    for (int i = 0; i < num_words; i++) {
        val[i] = (svBitVecVal)value_s.value.vector[i].aval;
    }
    vpi_free_object(h);
    return 1;
}

int backdoor_force_vec128(const char* path, const svBitVecVal* val) {
    vpiHandle h = get_handle(path);
    if (!h) return 0;
    
    int size = vpi_get(vpiSize, h);
    int num_words = (size + 31) / 32;
    s_vpi_vecval* vec_data = (s_vpi_vecval*)malloc(num_words * sizeof(s_vpi_vecval));
    for (int i = 0; i < num_words; i++) {
        vec_data[i].aval = val[i];
        vec_data[i].bval = 0;
    }

    s_vpi_value value_s;
    value_s.format = vpiVectorVal;
    value_s.value.vector = vec_data;
    
    s_vpi_time time_s = {vpiSimTime, 0, 0, 0.0};
    vpi_put_value(h, &value_s, &time_s, vpiForceFlag);
    free(vec_data);
    vpi_free_object(h);
    return 1;
}

// ----------------------------------------------------
// RELEASE API
// ----------------------------------------------------
int backdoor_release(const char* path) {
    vpiHandle h = get_handle(path);
    if (!h) return 0;
    
    // Pass a dummy value structure with vpiObjTypeVal format (uses target's native type format)
    // to satisfy strict simulator implementations (like Questa)
    s_vpi_value value_s;
    value_s.format = vpiObjTypeVal;
    vpi_put_value(h, &value_s, NULL, vpiReleaseFlag);
    
    vpi_free_object(h);
    return 1;
}
