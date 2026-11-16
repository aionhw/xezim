#include <stdio.h>
#include <string.h>
#include "vpi_user.h"
#include "sv_vpi_user.h"
extern vpiHandle vpi_register_assertion_cb(vpiHandle assertion, PLI_INT32 reason, vpi_assertion_callback_func *cb_rtn, PLI_BYTE8 *user_data) __attribute__((weak));
static void say(const char *s) { vpi_printf("V|%s\n", s); }
static PLI_INT32 vc_cb(p_cb_data d) {
  char b[128]; snprintf(b, sizeof b, "valuechange %s t=%u val=%d", vpi_get_str(vpiName, d->obj), d->time->low, d->value->value.integer); say(b); return 0;
}
static PLI_INT32 assert_cb(PLI_INT32 reason, p_vpi_time t, vpiHandle a, p_vpi_attempt_info info, PLI_BYTE8 *ud) {
  char b[128]; snprintf(b, sizeof b, "assertcb reason=%s name=%s t=%u", reason == cbAssertionFailure ? "fail" : "other", vpi_get_str(vpiName, a), t ? t->low : 0); say(b); return 0;
}
static PLI_INT32 step = 0;
static PLI_INT32 probe_calltf(PLI_BYTE8 *ud) {
  char b[256]; s_vpi_time t; t.type = vpiSimTime; vpi_get_time(NULL, &t);
  step++;
  if (step == 1) {
    vpiHandle top = vpi_handle_by_name("c36", NULL);
    snprintf(b, sizeof b, "top name=%s full=%s type=%d", vpi_get_str(vpiName, top), vpi_get_str(vpiFullName, top), vpi_get(vpiType, top)); say(b);
    int types[] = { vpiNet, vpiReg, vpiModule, vpiParameter };
    const char *tn[] = { "net", "reg", "module", "param" };
    for (int k = 0; k < 4; k++) {
      vpiHandle it = vpi_iterate(types[k], top), h; int n = 0; char names[200] = "";
      if (it) while ((h = vpi_scan(it))) { n++; strncat(names, vpi_get_str(vpiName, h), 190 - strlen(names)); strcat(names, ","); }
      snprintf(b, sizeof b, "iterate %s n=%d [%s]", tn[k], n, names); say(b);
    }
    vpiHandle a = vpi_handle_by_name("c36.a", NULL); s_vpi_value v; v.format = vpiIntVal; vpi_get_value(a, &v);
    snprintf(b, sizeof b, "get a=%d size=%d", v.value.integer, vpi_get(vpiSize, a)); say(b);
    vpiHandle w = vpi_handle_by_name("c36.u.w", NULL); v.format = vpiBinStrVal; vpi_get_value(w, &v);
    snprintf(b, sizeof b, "get u.w bin=%s", v.value.str); say(b);
    v.format = vpiIntVal; v.value.integer = 99; s_vpi_time dt; dt.type = vpiSimTime; dt.high = 0; dt.low = 5;
    vpi_put_value(a, &v, &dt, vpiTransportDelay);
    v.value.integer = 7; vpiHandle c = vpi_handle_by_name("c36.c", NULL); vpi_put_value(c, &v, NULL, vpiNoDelay);
    s_cb_data cb; s_vpi_value cv; s_vpi_time ct; ct.type = vpiSimTime; cv.format = vpiIntVal;
    memset(&cb, 0, sizeof cb); cb.reason = cbValueChange; cb.obj = vpi_handle_by_name("c36.cnt", NULL); cb.cb_rtn = vc_cb; cb.time = &ct; cb.value = &cv;
    vpi_register_cb(&cb);
    if (vpi_register_assertion_cb) {
      vpiHandle as = vpi_handle_by_name("c36.ap", NULL);
      snprintf(b, sizeof b, "assert handle %s", as ? "found" : "null"); say(b);
      if (as) vpi_register_assertion_cb(as, cbAssertionFailure, assert_cb, NULL);
    } else say("assert api absent");
  } else {
    snprintf(b, sizeof b, "time now=%u", t.low); say(b);
  }
  return 0;
}
static void reg(void) {
  s_vpi_systf_data tf; memset(&tf, 0, sizeof tf); tf.type = vpiSysTask; tf.tfname = "$probe_vpi"; tf.calltf = probe_calltf; vpi_register_systf(&tf);
}
void (*vlog_startup_routines[])(void) = { reg, 0 };
