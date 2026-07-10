// uvm_dpi_test.c — smoke test for the minimum VPI/DPI surface that
// xezim exposes for the Accellera UVM reference implementation.
//
// Each test function exercises one VPI/DPI primitive that UVM calls at
// startup or first use. The SystemVerilog side (uvm_dpi_test.sv) drives
// the calls via `import "DPI-C"` declarations and asserts the return
// values / side effects match expectations.
//
// xezim's VPI surface is intentionally minimal (see docs/dpi-guide.md
// for the full list). The functions here match exactly the prototypes
// that xezim implements.

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "svdpi.h"
#include "vpi_user.h"

// --- DPI version -----------------------------------------------------------

// xezim returns the static string "1800-2017" from svDpiVersion. UVM
// prints this in its banner; we just confirm the pointer is non-null
// and the string is non-empty.
const char *uvm_dpi_test_version(void) {
    const char *v = svDpiVersion();
    if (v == NULL || v[0] == '\0') {
        return "FAIL: empty version";
    }
    return v;
}

// --- vpi_get_vlog_info ------------------------------------------------------

// UVM's cmdline processor calls this exactly once at startup. We verify
// argc > 0 (the simulator always has at least the binary name in argv)
// and that argv[0] is non-null.
int uvm_dpi_test_vlog_info(int *out_argc) {
    s_vpi_vlog_info info;
    // Zero-init so a missing field is detectable in the future.
    memset(&info, 0, sizeof(info));
    if (vpi_get_vlog_info(&info) == 0) {
        return -1; // simulator refused to fill the struct
    }
    if (info.argc <= 0) {
        return -2; // argc should be at least 1 (argv[0])
    }
    if (info.argv == NULL) {
        return -3; // argv pointer missing
    }
    if (info.argv[0] == NULL) {
        return -4; // argv[0] missing
    }
    if (info.product == NULL || info.version == NULL) {
        return -5; // product/version missing
    }
    *out_argc = info.argc;
    return 0;
}

// --- scope round-trip -------------------------------------------------------

// UVM uses scope handles to identify which package a DPI import belongs
// to. The minimum contract is: a scope retrieved by name must hand back
// the same name via svGetNameFromScope.
//
// Returns the recovered name as a static buffer (small, since scope
// names are short — we cap at 256 chars; on overflow we truncate).
const char *uvm_dpi_test_scope_roundtrip(const char *name) {
    static char buf[256];
    buf[0] = '\0';
    void *scope = svGetScopeFromName(name);
    if (scope == NULL) {
        return buf; // empty -> SV side sees ""
    }
    const char *got = svGetNameFromScope(scope);
    if (got == NULL) {
        return buf;
    }
    int n = (int)strlen(got);
    if (n >= (int)sizeof(buf)) {
        n = (int)sizeof(buf) - 1;
    }
    memcpy(buf, got, n);
    buf[n] = '\0';
    // free the CString xezim leaked in svGetNameFromScope
    free((void *)got);
    return buf;
}

// svGetScope + svSetScope: the active scope must change and round-trip.
int uvm_dpi_test_scope_active(const char *name) {
    void *before = svGetScope();
    void *target = svGetScopeFromName(name);
    if (target == NULL) {
        return -1;
    }
    void *prev = svSetScope(target);
    void *now = svGetScope();
    int rc = (now == target) ? 0 : -2;
    // restore previous scope (so back-to-back calls are independent)
    svSetScope(prev);
    (void)before;
    return rc;
}

// --- vpi_register_cb / vpi_remove_cb ---------------------------------------
//
// Two reasons are supported: cbValueChange (6) and cbStartOfReset (15).
// We use static counters updated from the C callback so the SV side can
// read them back and assert they fire as expected.

static int g_value_change_count = 0;
static int g_reset_count = 0;
static int g_last_change_sig_id = -1;

// called by xezim's value-change dispatcher; we receive the user_data
// cookie (which is the signal id the SV side passed in) and bump the
// counter.
static PLI_INT32 uvm_dpi_test_value_change_cb(p_cb_data cb_data_p) {
    g_value_change_count++;
    if (cb_data_p != NULL && cb_data_p->user_data != NULL) {
        g_last_change_sig_id = *(int *)cb_data_p->user_data;
    }
    return 0;
}

static PLI_INT32 uvm_dpi_test_reset_cb(p_cb_data cb_data_p) {
    g_reset_count++;
    (void)cb_data_p;
    return 0;
}

// Register a value-change callback on a signal looked up by name. Returns
// the opaque handle on success, NULL on failure (e.g. signal not found).
void *uvm_dpi_test_register_value_change(const char *signal_name,
                                         int *out_sig_id) {
    vpiHandle h = vpi_handle_by_name((char *)signal_name, NULL);
    if (h == NULL) {
        return NULL;
    }
    int *cookie = (int *)malloc(sizeof(int));
    *cookie = (int)vpi_get(vpiSize, h);
    if (out_sig_id != NULL) {
        *out_sig_id = *cookie;
    }
    s_cb_data cb;
    memset(&cb, 0, sizeof(cb));
    cb.reason = cbValueChange;
    cb.cb_rtn = uvm_dpi_test_value_change_cb;
    cb.obj = h;
    cb.user_data = (PLI_BYTE8 *)cookie;
    void *reg = vpi_register_cb(&cb);
    // We intentionally keep `h` and `cookie` alive for the duration
    // of the test; in real UVM these are owned by the polling
    // framework and freed alongside the callback.
    return reg;
}

void *uvm_dpi_test_register_reset(void) {
    s_cb_data cb;
    memset(&cb, 0, sizeof(cb));
    cb.reason = cbStartOfReset;
    cb.cb_rtn = uvm_dpi_test_reset_cb;
    cb.obj = NULL;
    cb.user_data = NULL;
    return vpi_register_cb(&cb);
}

int uvm_dpi_test_remove_cb(void *handle) {
    if (handle == NULL) {
        return -1;
    }
    return vpi_remove_cb(handle);
}

int uvm_dpi_test_value_change_count(void) {
    return g_value_change_count;
}

int uvm_dpi_test_reset_count(void) {
    return g_reset_count;
}

// --- vpi_get smoke test ----------------------------------------------------

// Walk a signal and return (type, size, signed) from vpi_get. Returns
// 0 on success. The arguments are filled in place.
int uvm_dpi_test_vpi_get(const char *signal_name,
                         int *out_type,
                         int *out_size,
                         int *out_signed) {
    vpiHandle h = vpi_handle_by_name((char *)signal_name, NULL);
    if (h == NULL) {
        return -1;
    }
    *out_type = vpi_get(vpiType, h);
    *out_size = vpi_get(vpiSize, h);
    *out_signed = vpi_get(vpiSigned, h);
    return 0;
}