//----------------------------------------------------------------------
// xezim UVM DPI driver.
//
// Mirrors `uvm_dpi.cc` from the Accellera UVM reference but skips
// `uvm_hdl.c` entirely (its VCS / Questa / Xcelium branches all
// require proprietary vendor headers). The `uvm_hdl_*` surface is
// implemented directly here against standard IEEE 1800 VPI — no
// separate C file, no mangling concerns.
//
// Build:
//   g++ -shared -fPIC -std=c++17 -Wno-format-security \
//       -I path/to/xezim/include -I path/to/uvm-core/src/dpi \
//       path/to/xezim/include/uvm_dpi_xezim.cc \
//       -o uvm.so
//
// `-Wno-format-security` silences a long-standing warning from
// uvm_hdl_polling.c lines 526/533/534 where the Accellera UVM
// reference uses `sprintf(buf, str, name)` with a non-literal
// "format" string. That's technically UB if `str`/`name` ever
// contains `%`, but patching it in upstream uvm-core would be
// reverted on the next submodule update. Every commercial
// simulator's UVM build applies the same suppression.
//----------------------------------------------------------------------

// `uvm_dpi.h` declares its prototypes without `extern "C"`. When this
// file is compiled as C++ those prototypes default to C++ linkage.
// We need C linkage for the implementations below (so SV can resolve
// the symbols by their C names), so wrap the header include.
extern "C" {
#include "uvm_dpi.h"

// Forward declaration for `uvm_re_compexecfree` — the Accellera
// UVM reference declares it in `uvm_regex.svh` but the C prototype
// is missing from `uvm_dpi.h`. Without this forward decl, g++ sees
// the definition in `uvm_regex.cc` with no prior declaration in
// scope and mangles the symbol as `unsigned char (*)(const char*,
// const char*, unsigned char, int*)`. The SV `import "DPI-C"`
// dlsym lookup then fails with "unresolved symbol".
unsigned char uvm_re_compexecfree(const char* re, const char* str,
                                   unsigned char deglob, int* exec_ret);
}

#include <string.h>

//----------------------------------------------------------------------
// uvm_hdl_* — IEEE 1800.2-2017 Annex C (UVM HDL Access).
//
// Signatures and return values match the standard:
//   uvm_hdl_check_path       returns 1 if path exists, 0 otherwise
//   uvm_hdl_deposit          returns 1 on success, 0 on failure
//   uvm_hdl_force            returns 1 on success, 0 on failure
//   uvm_hdl_release          returns 1 on success, 0 on failure
//   uvm_hdl_release_and_read returns 1 on success, 0 on failure
//   uvm_hdl_read             returns 1 on success, 0 on failure
//
// Questa-specific helpers (`uvm_is_vhdl_path`,
// `uvm_register_get_vhdl`, `uvm_register_set_vhdl`) are NOT part
// of IEEE 1800.2 and are intentionally not provided.
//----------------------------------------------------------------------

// Defensive upper bound on vector words we'll accept (4096 words
// = 16384 bits). UVM's default max width is 1024 bits.
#define XEZIM_VECVAL_MAX_WORDS 4096

// Write the vector `value` to `path` using the given vpi_put_value
// flag (vpiNoDelay / vpiForceFlag / vpiReleaseFlag). Read-modify-
// write for write flags so callers passing fewer words than the
// signal width don't accidentally zero the upper bits.
static int uvm_hdl_set_vlog(char *path, p_vpi_vecval value, int flag) {
    vpiHandle h = vpi_handle_by_name(path, NULL);
    if (h == NULL) return 0;

    s_vpi_value value_s;

    PLI_INT32 size = vpi_get(vpiSize, h);
    int words = (size + 31) / 32;
    if (size <= 0 || words > XEZIM_VECVAL_MAX_WORDS) {
        vpi_free_object(h);
        return 0;
    }

    if (flag == vpiReleaseFlag) {
        // Release: pass a vpiObjTypeVal placeholder so the simulator
        // uses the target's native type format.
        value_s.format = vpiObjTypeVal;
        vpi_put_value(h, &value_s, NULL, vpiReleaseFlag);
        vpi_free_object(h);
        return 1;
    }

    // vpi_get_value points value_s.value.vector at SIMULATOR-owned
    // storage, valid only until the next vpi_get_value call. Copy it out
    // before touching it, and before vpi_put_value can reuse the buffer.
    s_vpi_vecval cur[XEZIM_VECVAL_MAX_WORDS];
    memset(cur, 0, sizeof(cur));
    value_s.format = vpiVectorVal;
    vpi_get_value(h, &value_s);
    if (value_s.format == vpiVectorVal && value_s.value.vector != NULL) {
        memcpy(cur, value_s.value.vector, (size_t)words * sizeof(s_vpi_vecval));
    }

    for (int i = 0; i < words; i++) {
        cur[i].aval = value[i].aval;
        cur[i].bval = value[i].bval;
    }

    value_s.format = vpiVectorVal;
    value_s.value.vector = cur;
    vpi_put_value(h, &value_s, NULL, flag);
    vpi_free_object(h);
    return 1;
}

// Read the vector at `path` into `value`.
//
// vpi_get_value does NOT fill a caller-supplied buffer: it points
// value_s.value.vector at its own storage (IEEE 1800-2017 §38.16). This
// used to assign `value` into that field and then return success without
// reading anything back, so uvm_hdl_read reported success while leaving
// the caller's buffer exactly as it found it.
static int uvm_hdl_get_vlog(char *path, p_vpi_vecval value) {
    vpiHandle h = vpi_handle_by_name(path, NULL);
    if (h == NULL) return 0;

    PLI_INT32 size = vpi_get(vpiSize, h);
    int words = (size + 31) / 32;
    if (size <= 0 || words > XEZIM_VECVAL_MAX_WORDS) {
        vpi_free_object(h);
        return 0;
    }

    s_vpi_value value_s;
    value_s.format = vpiVectorVal;
    vpi_get_value(h, &value_s);

    // vpiSuppressVal is the only failure channel vpi_get_value has.
    if (value_s.format != vpiVectorVal || value_s.value.vector == NULL) {
        vpi_free_object(h);
        return 0;
    }
    memcpy(value, value_s.value.vector, (size_t)words * sizeof(s_vpi_vecval));
    vpi_free_object(h);
    return 1;
}

extern "C" {

int uvm_hdl_check_path(char *path) {
    if (path == NULL) return 0;
    vpiHandle h = vpi_handle_by_name(path, NULL);
    if (h == NULL) return 0;
    vpi_free_object(h);
    return 1;
}

int uvm_hdl_read(char *path, p_vpi_vecval value) {
    if (path == NULL || value == NULL) return 0;
    return uvm_hdl_get_vlog(path, value);
}

int uvm_hdl_deposit(char *path, p_vpi_vecval value) {
    if (path == NULL || value == NULL) return 0;
    return uvm_hdl_set_vlog(path, value, vpiNoDelay);
}

int uvm_hdl_force(char *path, p_vpi_vecval value) {
    if (path == NULL || value == NULL) return 0;
    return uvm_hdl_set_vlog(path, value, vpiForceFlag);
}

int uvm_hdl_release(char *path) {
    if (path == NULL) return 0;
    return uvm_hdl_set_vlog(path, NULL, vpiReleaseFlag);
}

int uvm_hdl_release_and_read(char *path, p_vpi_vecval value) {
    if (path == NULL) return 0;
    if (value != NULL) uvm_hdl_get_vlog(path, value);
    return uvm_hdl_set_vlog(path, NULL, vpiReleaseFlag);
}

}  // extern "C"

//----------------------------------------------------------------------
// UVM C/C++ sources. Same include order as `uvm_dpi.cc` but with
// `uvm_hdl.c` skipped — its `uvm_hdl_*` surface is implemented above.
//----------------------------------------------------------------------

#include "uvm_common.c"
#include "uvm_regex.cc"
#include "uvm_svcmd_dpi.c"

// uvm_hdl_polling.c ships with uvm-core and 1800.2-2020.3.1, but is
// absent from 1800.2-2017-1.0 (whose uvm_dpi.cc has this include
// `#ifdef UVM_PLI_POLLING_ENABLE`'d out by default). Define
// XEZIM_UVM_POLLING via -D when the source dir provides the file.
#ifdef XEZIM_UVM_POLLING
#include "uvm_hdl_polling.c"
#endif