/* The remaining IEEE 1800-2017 clause 38 surface: a registered $systf reading
 * its own arguments (vpiSysTfCall / vpiArgument), writing an output argument,
 * a system FUNCTION returning a value, vpi_chk_error, and vpi_control.
 *
 * Before this, `vpi_register_systf` accepted a vpiSysFunc but never dispatched
 * one, a task could not see its arguments at all, every failure was invisible
 * to `vpi_chk_error` (it always answered "no error"), and `vpi_control` did
 * not exist.
 */
#include <stdio.h>
#include <string.h>
#include "vpi_user.h"

static int errors = 0;

#define CHECK(cond, msg)                                                    \
    do {                                                                    \
        if (!(cond)) {                                                      \
            vpi_printf("FAIL: %s\n", (msg));                                \
            errors++;                                                       \
        }                                                                   \
    } while (0)

/* $st_args(sig, 42, other) — read every argument. */
static PLI_INT32 st_args(PLI_BYTE8 *ud) {
    (void)ud;
    vpiHandle call = vpi_handle(vpiSysTfCall, NULL);
    CHECK(call != NULL, "vpi_handle(vpiSysTfCall, NULL) inside a calltf");
    CHECK(vpi_get(vpiType, call) == vpiSysTaskCall, "a task call is vpiSysTaskCall");
    CHECK(strcmp(vpi_get_str(vpiName, call), "$st_args") == 0, "call vpiName");

    vpiHandle it = vpi_iterate(vpiArgument, call);
    CHECK(it != NULL, "vpi_iterate(vpiArgument, call)");

    vpiHandle a;
    int n = 0;
    int vals[3] = {0, 0, 0};
    int types[3] = {0, 0, 0};
    while ((a = vpi_scan(it)) != NULL && n < 3) {
        s_vpi_value v;
        v.format = vpiIntVal;
        vpi_get_value(a, &v);
        vals[n] = v.value.integer;
        types[n] = vpi_get(vpiType, a);
        n++;
        vpi_free_object(a);
    }
    CHECK(n == 3, "three arguments");
    CHECK(vals[0] == 7 && vals[1] == 42 && vals[2] == 5, "argument values");
    /* A signal-backed argument is a real object; a literal is a vpiConstant. */
    CHECK(types[0] == vpiIntVar, "a signal argument keeps its type");
    CHECK(types[1] == vpiConstant, "a literal argument is a vpiConstant");

    vpi_free_object(call);
    return 0;
}

/* $st_bump(sig) — write through a signal-backed (output) argument. */
static PLI_INT32 st_bump(PLI_BYTE8 *ud) {
    (void)ud;
    vpiHandle call = vpi_handle(vpiSysTfCall, NULL);
    vpiHandle it = vpi_iterate(vpiArgument, call);
    vpiHandle a = vpi_scan(it);
    s_vpi_value v;
    v.format = vpiIntVal;
    vpi_get_value(a, &v);
    v.value.integer += 100;
    vpi_put_value(a, &v, NULL, vpiNoDelay);
    while (vpi_scan(it) != NULL) {} /* drain, so the iterator frees itself */
    vpi_free_object(a);
    vpi_free_object(call);
    return 0;
}

/* $st_const(42) — a vpiConstant argument must be read-only. */
static PLI_INT32 st_const(PLI_BYTE8 *ud) {
    (void)ud;
    (void)vpi_chk_error(NULL); /* clear anything pending */
    vpiHandle call = vpi_handle(vpiSysTfCall, NULL);
    vpiHandle it = vpi_iterate(vpiArgument, call);
    vpiHandle a = vpi_scan(it);
    while (vpi_scan(it) != NULL) {}
    s_vpi_value v;
    v.format = vpiIntVal;
    v.value.integer = 1;
    vpi_put_value(a, &v, NULL, vpiNoDelay);
    CHECK(vpi_chk_error(NULL) == vpiError, "writing a vpiConstant must be an error");
    vpi_free_object(a);
    vpi_free_object(call);
    return 0;
}

/* $st_triple(x) — a system FUNCTION returning 3*x. */
static PLI_INT32 st_triple(PLI_BYTE8 *ud) {
    (void)ud;
    vpiHandle call = vpi_handle(vpiSysTfCall, NULL);
    CHECK(vpi_get(vpiType, call) == vpiSysFuncCall, "a function call is vpiSysFuncCall");
    CHECK(vpi_get(vpiFuncType, call) == vpiIntFunc, "vpiFuncType is the registered sysfunctype");
    CHECK(vpi_get(vpiSize, call) == 32, "vpiIntFunc returns 32 bits");

    vpiHandle it = vpi_iterate(vpiArgument, call);
    vpiHandle a = vpi_scan(it);
    s_vpi_value v;
    v.format = vpiIntVal;
    vpi_get_value(a, &v);
    while (vpi_scan(it) != NULL) {}
    vpi_free_object(a);

    s_vpi_value r;
    r.format = vpiIntVal;
    r.value.integer = v.value.integer * 3;
    vpi_put_value(call, &r, NULL, vpiNoDelay);
    vpi_free_object(call);
    return 0;
}

/* $st_silent() — a function that deposits nothing returns 0. */
static PLI_INT32 st_silent(PLI_BYTE8 *ud) {
    (void)ud;
    return 0;
}

/* $st_sized() — vpiSizedFunc asks sizetf for its width. */
static PLI_INT32 st_sized_size(PLI_BYTE8 *ud) {
    (void)ud;
    return 8;
}
static PLI_INT32 st_sized(PLI_BYTE8 *ud) {
    (void)ud;
    vpiHandle call = vpi_handle(vpiSysTfCall, NULL);
    CHECK(vpi_get(vpiSize, call) == 8, "sizetf sets the return width");
    s_vpi_value r;
    r.format = vpiIntVal;
    r.value.integer = 0x1FF; /* must truncate to 8 bits -> 0xFF */
    vpi_put_value(call, &r, NULL, vpiNoDelay);
    vpi_free_object(call);
    return 0;
}

/* $st_errors() — vpi_chk_error reports, then clears. */
static PLI_INT32 st_errors(PLI_BYTE8 *ud) {
    (void)ud;
    (void)vpi_chk_error(NULL);
    CHECK(vpi_chk_error(NULL) == 0, "no error pending");

    /* An unsupported format is a failure vpi_get_value can only report by
     * setting vpiSuppressVal — vpi_chk_error must see it too. */
    vpiHandle h = vpi_handle_by_name("tb.a", NULL);
    s_vpi_value v;
    v.format = vpiStrengthVal;
    vpi_get_value(h, &v);
    CHECK(v.format == vpiSuppressVal, "unsupported format sets vpiSuppressVal");

    s_vpi_error_info info;
    memset(&info, 0, sizeof info);
    int lvl = vpi_chk_error(&info);
    CHECK(lvl == vpiError, "vpi_chk_error reports the level");
    CHECK(info.state == vpiRun, "error state");
    CHECK(info.product && strcmp(info.product, "xezim") == 0, "error product");
    CHECK(info.message != NULL && strlen(info.message) > 0, "error message");
    CHECK(vpi_chk_error(NULL) == 0, "vpi_chk_error clears the error");

    /* A NULL handle is an error too, not a silent no-op. */
    v.format = vpiIntVal;
    vpi_get_value(NULL, &v);
    CHECK(vpi_chk_error(NULL) == vpiError, "a NULL handle is reported");

    vpi_free_object(h);
    return 0;
}

/* $st_report() — print the tally, then end the run through vpi_control. */
static PLI_INT32 st_report(PLI_BYTE8 *ud) {
    (void)ud;
    vpi_printf("SYSTF_ERRORS: %d\n", errors);
    return 0;
}

static PLI_INT32 st_finish(PLI_BYTE8 *ud) {
    (void)ud;
    CHECK(vpi_control(vpiReset) == 0, "vpiReset must be rejected");
    (void)vpi_chk_error(NULL);
    vpi_printf("BEFORE_FINISH\n");
    vpi_control(vpiFinish, 1);
    return 0;
}

static void tf(PLI_INT32 type, PLI_INT32 ftype, const char *name,
               PLI_INT32 (*calltf)(PLI_BYTE8 *), PLI_INT32 (*sizetf)(PLI_BYTE8 *)) {
    s_vpi_systf_data d;
    memset(&d, 0, sizeof d);
    d.type = type;
    d.sysfunctype = ftype;
    d.tfname = (PLI_BYTE8 *)name;
    d.calltf = calltf;
    d.sizetf = sizetf;
    vpi_register_systf(&d);
}

static void systf_register(void) {
    tf(vpiSysTask, 0, "$st_args", st_args, NULL);
    tf(vpiSysTask, 0, "$st_bump", st_bump, NULL);
    tf(vpiSysTask, 0, "$st_const", st_const, NULL);
    tf(vpiSysTask, 0, "$st_errors", st_errors, NULL);
    tf(vpiSysTask, 0, "$st_report", st_report, NULL);
    tf(vpiSysTask, 0, "$st_finish", st_finish, NULL);
    tf(vpiSysFunc, vpiIntFunc, "$st_triple", st_triple, NULL);
    tf(vpiSysFunc, vpiIntFunc, "$st_silent", st_silent, NULL);
    tf(vpiSysFunc, vpiSizedFunc, "$st_sized", st_sized, st_sized_size);
}

void (*vlog_startup_routines[])(void) = { systf_register, 0 };
