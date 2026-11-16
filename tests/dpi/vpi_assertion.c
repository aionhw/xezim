/* IEEE 1800-2017 Clause 39, the assertion API, as an assertion unit-test
 * framework drives it:
 *
 *   - 39.3: the assertions of a scope are objects, reached by
 *     vpi_iterate(vpiAssertion, scope), with the Annex M vpiType spellings;
 *   - 39.4.2: vpi_register_assertion_cb delivers cbAssertionStart, Success
 *     and Failure with the attempt's start time in s_vpi_attempt_info, and
 *     vpi_remove_cb removes such a callback;
 *   - 39.5: vpi_control(vpiAssertionDisable / vpiAssertionEnable) stops and
 *     resumes new attempts, reporting cbAssertionDisable / Enable; and
 *     vpiAssertionKill discards the attempt started at a given time,
 *     reporting cbAssertionKill, while the assertion stays enabled;
 *   - Clause 40: vpiAssertSuccessCovered / vpiAssertFailureCovered.
 *
 * None of this existed: there was no vpi_register_assertion_cb, no
 * assertion object and no assertion operation of vpi_control.
 *
 * The module prints one `CB` line per callback; the Rust test checks the
 * transcript.
 */
#include <stdio.h>
#include <string.h>
#include "vpi_user.h"
#include "sv_vpi_user.h"

static int errors = 0;

#define CHECK(cond, msg)                                                    \
    do {                                                                    \
        if (!(cond)) {                                                      \
            vpi_printf("FAIL: %s\n", (msg));                                \
            errors++;                                                       \
        }                                                                   \
    } while (0)

#define MAX_OBJS 16
static vpiHandle objs[MAX_OBJS];
static int n_objs = 0;

static const char *reason_name(PLI_INT32 r) {
    switch (r) {
    case cbAssertionStart: return "START";
    case cbAssertionSuccess: return "SUCCESS";
    case cbAssertionFailure: return "FAILURE";
    case cbAssertionDisable: return "DISABLE";
    case cbAssertionEnable: return "ENABLE";
    case cbAssertionReset: return "RESET";
    case cbAssertionKill: return "KILL";
    default: return "OTHER";
    }
}

static PLI_INT32 assertion_cb(PLI_INT32 reason, p_vpi_time cb_time,
                              vpiHandle assertion, p_vpi_attempt_info info,
                              PLI_BYTE8 *user_data) {
    (void)user_data;
    CHECK(cb_time != NULL, "cb_time is set");
    CHECK(info != NULL, "info is set");
    CHECK(cb_time == NULL || cb_time->type == vpiSimTime, "cb_time is vpiSimTime");
    vpi_printf("CB %s %s t=%u start=%u\n", vpi_get_str(vpiName, assertion),
               reason_name(reason), cb_time ? cb_time->low : 0u,
               info ? info->attemptStartTime.low : 0u);
    return 0;
}

static vpiHandle find(const char *name, PLI_INT32 type) {
    for (int i = 0; i < n_objs; i++) {
        if (vpi_get(vpiType, objs[i]) == type &&
            strcmp(vpi_get_str(vpiName, objs[i]), name) == 0)
            return objs[i];
    }
    return NULL;
}

static vpiHandle reg(vpiHandle a, PLI_INT32 reason) {
    vpiHandle cb = vpi_register_assertion_cb(a, reason, assertion_cb, NULL);
    CHECK(cb != NULL, "vpi_register_assertion_cb returns a handle");
    return cb;
}

/* $va_setup: enumerate the assertions of `top` and register callbacks. */
static PLI_INT32 va_setup(PLI_BYTE8 *ud) {
    (void)ud;
    vpiHandle top = vpi_handle_by_name("top", NULL);
    CHECK(top != NULL, "vpi_handle_by_name(top)");
    vpiHandle it = vpi_iterate(vpiAssertion, top);
    CHECK(it != NULL, "vpi_iterate(vpiAssertion, top)");
    vpiHandle o;
    while (it && (o = vpi_scan(it)) != NULL && n_objs < MAX_OBJS) {
        vpi_printf("OBJ %s %s\n", vpi_get_str(vpiType, o),
                   vpi_get_str(vpiFullName, o));
        objs[n_objs++] = o;
    }
    vpiHandle a1 = find("A1", vpiAssert);
    vpiHandle a2 = find("A2", vpiAssert);
    vpiHandle c1 = find("C1", vpiCover);
    vpiHandle pd = find("p_a", vpiPropertyDecl);
    CHECK(a1 && a2 && c1 && pd, "A1, A2, C1 and p_a are enumerated");
    if (!(a1 && a2 && c1 && pd)) return 0;

    PLI_INT32 a1_reasons[] = {cbAssertionStart, cbAssertionSuccess,
                              cbAssertionFailure, cbAssertionDisable,
                              cbAssertionEnable, cbAssertionReset,
                              cbAssertionKill};
    for (unsigned i = 0; i < sizeof a1_reasons / sizeof a1_reasons[0]; i++)
        reg(a1, a1_reasons[i]);
    reg(a2, cbAssertionSuccess);
    reg(a2, cbAssertionFailure);
    reg(a2, cbAssertionKill);
    /* A declaration has no attempts: registering is accepted, nothing fires. */
    reg(pd, cbAssertionStart);
    /* A removed callback never fires (A1 would report SUCCESS twice). */
    vpiHandle extra = reg(a1, cbAssertionSuccess);
    CHECK(vpi_remove_cb(extra) == 1, "vpi_remove_cb of an assertion callback");
    /* Not an assertion callback reason. */
    CHECK(vpi_register_assertion_cb(a1, cbValueChange, assertion_cb, NULL) == NULL,
          "cbValueChange is not an assertion callback reason");
    vpi_chk_error(NULL);
    /* Phase 2 lock and action reasons are refused (Clause 39.4.2). */
    CHECK(vpi_register_assertion_cb(a1, cbAssertionLock, assertion_cb, NULL) == NULL,
          "cbAssertionLock is not supported");
    vpi_chk_error(NULL);
    CHECK(vpi_register_assertion_cb(a1, cbAssertionEnablePassAction, assertion_cb, NULL) == NULL,
          "cbAssertionEnablePassAction is not supported");
    vpi_chk_error(NULL);
    return 0;
}

/* $va_ctl(op, "name", start): one assertion operation of vpi_control. */
static PLI_INT32 va_ctl(PLI_BYTE8 *ud) {
    (void)ud;
    vpiHandle call = vpi_handle(vpiSysTfCall, NULL);
    vpiHandle it = vpi_iterate(vpiArgument, call);
    s_vpi_value v;
    v.format = vpiIntVal;
    vpi_get_value(vpi_scan(it), &v);
    PLI_INT32 op = v.value.integer;
    v.format = vpiStringVal;
    vpi_get_value(vpi_scan(it), &v);
    char name[64];
    strncpy(name, v.value.str, sizeof name - 1);
    name[sizeof name - 1] = '\0';
    v.format = vpiIntVal;
    vpi_get_value(vpi_scan(it), &v);
    PLI_INT32 start = v.value.integer;
    vpi_free_object(it);

    vpiHandle a = find(name, vpiAssert);
    CHECK(a != NULL, "the controlled assertion is enumerated");
    if (!a) return 0;
    if (op == vpiAssertionKill) {
        s_vpi_time t;
        t.type = vpiSimTime;
        t.high = 0;
        t.low = (PLI_UINT32)start;
        t.real = 0.0;
        vpi_control(op, a, &t);
    } else {
        vpi_control(op, a);
    }
    return 0;
}

/* $va_check: the coverage counts, then the verdict. */
static PLI_INT32 va_check(PLI_BYTE8 *ud) {
    (void)ud;
    vpiHandle c1 = find("C1", vpiCover);
    if (c1) {
        vpi_printf("COVER C1 success=%d failure=%d\n",
                   (int)vpi_get(vpiAssertSuccessCovered, c1),
                   (int)vpi_get(vpiAssertFailureCovered, c1));
    }
    vpi_printf("VA_ERRORS: %d\n", errors);
    return 0;
}

static void va_register(void) {
    s_vpi_systf_data d;
    memset(&d, 0, sizeof d);
    d.type = vpiSysTask;
    d.tfname = "$va_setup";
    d.calltf = va_setup;
    vpi_register_systf(&d);
    d.tfname = "$va_ctl";
    d.calltf = va_ctl;
    vpi_register_systf(&d);
    d.tfname = "$va_check";
    d.calltf = va_check;
    vpi_register_systf(&d);
}

void (*vlog_startup_routines[])(void) = { va_register, 0 };
