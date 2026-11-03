/* The VPI side of vpi_top.vhd: writes d at 1 ns, reads q at 2 ns and en
 * at 3 ns, after writing it to 0 at 2.5 ns. */
#include <stdio.h>
#include "vpi_user.h"

static void after(PLI_INT32 (*f)(p_cb_data), unsigned fs) {
  s_vpi_time t = {vpiSimTime, 0, fs, 0.0};
  s_cb_data cb = {cbAfterDelay, f, NULL, &t, NULL, 0, NULL};
  vpi_free_object(vpi_register_cb(&cb));
}

static void put(const char *name, PLI_INT32 fmt, const char *s, PLI_INT32 i) {
  vpiHandle h = vpi_handle_by_name((PLI_BYTE8 *)name, NULL);
  if (!h) { vpi_printf("vpi: no %s\n", name); return; }
  s_vpi_value v;
  v.format = fmt;
  if (fmt == vpiIntVal) v.value.integer = i; else v.value.str = (PLI_BYTE8 *)s;
  vpi_put_value(h, &v, NULL, vpiNoDelay);
}

static void show(const char *name) {
  vpiHandle h = vpi_handle_by_name((PLI_BYTE8 *)name, NULL);
  if (!h) { vpi_printf("vpi: no %s\n", name); return; }
  s_vpi_value v = {vpiBinStrVal};
  vpi_get_value(h, &v);
  vpi_printf("vpi: %s=%s\n", name, v.value.str);
}

static PLI_INT32 at3(p_cb_data cb) { (void)cb; show("vpi_top.en"); show("vpi_top.q"); return 0; }
static PLI_INT32 at25(p_cb_data cb) { (void)cb; put("vpi_top.en", vpiBinStrVal, "0", 0); after(at3, 500000); return 0; }
static PLI_INT32 at2(p_cb_data cb) { (void)cb; show("vpi_top.q"); after(at25, 500000); return 0; }
static PLI_INT32 at1(p_cb_data cb) { (void)cb; put("vpi_top.d", vpiIntVal, NULL, 0x5A); after(at2, 1000000); return 0; }
static PLI_INT32 start(p_cb_data cb) {
  (void)cb;
  vpi_printf("vpi: io %s\n", vpi_handle_by_name((PLI_BYTE8 *)"vpi_top.io", NULL) ? "visible" : "not visible");
  after(at1, 1000000);
  return 0;
}
static void reg(void) {
  s_cb_data cb = {cbStartOfSimulation, start, NULL, NULL, NULL, 0, NULL};
  vpi_free_object(vpi_register_cb(&cb));
}
void (*vlog_startup_routines[])(void) = {reg, 0};
