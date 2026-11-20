/* Walk the unpacked-array ports of tests/dpi/vpi_array_elem_range.sv the
 * way cocotb's GPI does (see tests/strings/vpi_array_elem_range.rs): the
 * array's range, an element by index, the element's range read through
 * vpi_handle(vpiLeftRange / vpiRightRange) and vpi_get_value, a write to
 * every element, and the error record of a failed call. Prints `CHK|...`
 * per check that holds and `FAIL: ...` otherwise. */
#include <stdio.h>
#include <string.h>
#include "vpi_user.h"
#include "sv_vpi_user.h"

#define CHECK(c, what)                          \
    do {                                        \
        if (c) vpi_printf("CHK|%s\n", what);    \
        else vpi_printf("FAIL: %s\n", what);    \
    } while (0)

/* The bounds of `h` as cocotb reads them: through its first vpiRange when
 * it has one, else through vpiLeftRange / vpiRightRange of `h` itself. */
static int bounds(vpiHandle h, int *l, int *r) {
    vpiHandle it = vpi_iterate(vpiRange, h), rh = h, lh, hh;
    s_vpi_value v;
    if (it) {
        rh = vpi_scan(it);
        vpi_free_object(it);
        if (!rh) return 0;
    }
    lh = vpi_handle(vpiLeftRange, rh);
    hh = vpi_handle(vpiRightRange, rh);
    if (!lh || !hh) return 0;
    v.format = vpiIntVal;
    vpi_get_value(lh, &v);
    *l = v.value.integer;
    v.format = vpiIntVal;
    vpi_get_value(hh, &v);
    *r = v.value.integer;
    return 1;
}

static void check_bounds(vpiHandle h, int l, int r, const char *what) {
    int gl = -99, gr = -99;
    char msg[128];
    int ok = h && bounds(h, &gl, &gr) && gl == l && gr == r;
    snprintf(msg, sizeof msg, "%s [%d:%d]%s", what, l, r, ok ? "" : " (wrong)");
    if (!ok) vpi_printf("  %s read [%d:%d]\n", what, gl, gr);
    CHECK(ok, msg);
}

static PLI_INT32 start(p_cb_data d) {
    vpiHandle ids, down, grid, e;
    s_vpi_error_info info;
    s_vpi_value v;
    int i;
    (void)d;
    ids = vpi_handle_by_name("top.ids", NULL);
    down = vpi_handle_by_name("top.down", NULL);
    grid = vpi_handle_by_name("top.grid", NULL);
    CHECK(ids && down && grid, "array handles");
    if (!ids || !down || !grid) return 0;
    check_bounds(ids, 0, 3, "ids");
    check_bounds(down, 3, 0, "down");
    check_bounds(grid, 0, 1, "grid");
    for (i = 0; i < 4; i++) {
        char what[32];
        e = vpi_handle_by_index(ids, i);
        snprintf(what, sizeof what, "ids[%d]", i);
        check_bounds(e, 31, 0, what);
        if (!e) continue;
        v.format = vpiIntVal;
        v.value.integer = 0x10 << i;
        vpi_put_value(e, &v, NULL, vpiNoDelay);
    }
    check_bounds(vpi_handle_by_index(down, 2), 0, 7, "down[2]");
    e = vpi_handle_by_index(grid, 1);
    check_bounds(e, 2, 0, "grid[1]");
    check_bounds(e ? vpi_handle_by_index(e, 0) : NULL, 15, 8, "grid[1][0]");
    /* A bit has no range. */
    e = vpi_handle_by_index(vpi_handle_by_index(ids, 0), 3);
    CHECK(e && vpi_handle(vpiLeftRange, e) == NULL, "ids[0][3] has no left range");
    vpi_chk_error(NULL);

    /* A failed call leaves an error record whose strings are all strings. */
    v.format = vpiIntVal;
    vpi_get_value(NULL, &v);
    memset(&info, 0, sizeof info);
    CHECK(vpi_chk_error(&info) == vpiError, "vpi_get_value(NULL) is an error");
    CHECK(info.message && info.product && info.code && info.file, "error strings are set");
    if (info.message && info.product && info.code && info.file)
        CHECK(strlen(info.message) > 0 && strlen(info.product) > 0 && strlen(info.file) == 0,
              "error strings read");
    return 0;
}

static PLI_INT32 finish(p_cb_data d) {
    s_vpi_value v;
    (void)d;
    v.format = vpiIntVal;
    vpi_get_value(vpi_handle_by_name("top.sum", NULL), &v);
    CHECK(v.value.integer == 0xf0, "sum of the written elements");
    return 0;
}

static void reg(void) {
    s_cb_data cb;
    memset(&cb, 0, sizeof cb);
    cb.reason = cbStartOfSimulation;
    cb.cb_rtn = start;
    vpi_register_cb(&cb);
    memset(&cb, 0, sizeof cb);
    cb.reason = cbEndOfSimulation;
    cb.cb_rtn = finish;
    vpi_register_cb(&cb);
}

void (*vlog_startup_routines[])(void) = {reg, 0};
