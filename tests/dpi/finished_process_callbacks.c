/* IEEE 1800-2023 §38.36: retain process/systf handles across fork exits. */
#include <stdio.h>
#include <string.h>
#include "vpi_user.h"
static int tags, statements;
static vpiHandle initial;
static PLI_INT32 disabled(p_cb_data cb) {
    s_vpi_time t = {0}; t.type = vpiSimTime; vpi_get_time(NULL, &t);
    vpi_printf("FORK_CB tag=%d t=%u type=%d\n", (int)(size_t)cb->user_data, t.low, vpi_get(vpiType, cb->obj));
    return 0;
}
static PLI_INT32 watch(PLI_BYTE8 *unused) {
    (void)unused;
    s_cb_data cb = {0};
    cb.reason = cbDisable; cb.cb_rtn = disabled;
    cb.obj = vpi_handle(vpiSysTfCall, NULL);
    cb.user_data = (PLI_BYTE8 *)(size_t)++tags;
    if (!vpi_register_cb(&cb)) vpi_printf("CALLBACK_REGISTRATION_FAILED\n");
    return 0;
}
static PLI_INT32 stmt(p_cb_data cb) {
    (void)cb; statements++;
    return 0;
}
static PLI_INT32 end(p_cb_data cb) {
    (void)cb;
    vpi_printf("FORK_CB_END tags=%d stmts=%d initial_type=%d\n", tags, statements, vpi_get(vpiType, initial));
    return 0;
}
static PLI_INT32 start(p_cb_data unused) {
    (void)unused;
    vpiHandle top = vpi_handle_by_name("top", NULL);
    vpiHandle it = vpi_iterate(vpiProcess, top);
    if (it) initial = vpi_scan(it);
    s_cb_data cb = {0}; cb.reason = cbStmt; cb.cb_rtn = stmt; cb.obj = top;
    vpi_register_cb(&cb);
    cb.reason = cbEndOfSimulation; cb.cb_rtn = end; cb.obj = NULL;
    vpi_register_cb(&cb);
    return 0;
}
static void install(void) {
    s_vpi_systf_data tf = {0};
    tf.type = vpiSysTask; tf.tfname = "$watch"; tf.calltf = watch; vpi_register_systf(&tf);
    s_cb_data cb = {0}; cb.reason = cbStartOfSimulation; cb.cb_rtn = start;
    vpi_register_cb(&cb);
}
void (*vlog_startup_routines[])(void) = {install, NULL};
