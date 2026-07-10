/* Regression coverage for the classic VPI surface: a module loaded with
 * `--vpi-lib`, registering a $systf through vlog_startup_routines, then
 * walking the design with vpi_iterate/vpi_scan/vpi_get_str.
 *
 * Before this existed xezim could not run a VPI application at all: there
 * was no module-loading path, no vpi_register_systf, and no object
 * traversal. The design is flattened at elaboration, so the instance tree
 * is reconstructed from ElaboratedModule::instances.
 *
 * Note on vpiInternalScope: per IEEE 1800-2017 it yields the internal
 * SCOPES of a module (child instances, named blocks), NOT its declared
 * nets and variables. Those come from vpiNet / vpiReg / vpiVariables /
 * vpiParameter / vpiMemory. A lot of VPI code in the wild gets this wrong.
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

/* Collect the names an iterator yields, comma-separated and sorted by the
 * simulator (iteration order is deterministic). */
static void collect(PLI_INT32 rel, vpiHandle scope, char *out, size_t cap) {
    out[0] = '\0';
    vpiHandle it = vpi_iterate(rel, scope);
    if (!it) return;
    vpiHandle o;
    while ((o = vpi_scan(it)) != NULL) {
        const char *n = vpi_get_str(vpiName, o);
        if (out[0]) strncat(out, ",", cap - strlen(out) - 1);
        strncat(out, n ? n : "?", cap - strlen(out) - 1);
        vpi_free_object(o);
    }
}

static PLI_INT32 om_check(PLI_BYTE8 *user_data) {
    char buf[256];

    /* user_data must survive registration. */
    CHECK(user_data != NULL && strcmp(user_data, "cookie") == 0, "systf user_data");

    /* --- the top module --- */
    vpiHandle top = vpi_handle(vpiScope, NULL);
    CHECK(top != NULL, "vpi_handle(vpiScope, NULL) yields the top module");
    CHECK(vpi_get(vpiType, top) == vpiModule, "top is a vpiModule");
    CHECK(strcmp(vpi_get_str(vpiName, top), "tb") == 0, "top vpiName");
    CHECK(vpi_handle(vpiScope, top) == NULL, "the top module has no parent scope");

    /* The standard route to the top: iterate vpiModule from NULL. */
    vpiHandle it = vpi_iterate(vpiModule, NULL);
    CHECK(it != NULL, "vpi_iterate(vpiModule, NULL)");
    vpiHandle t2 = vpi_scan(it);
    CHECK(t2 && strcmp(vpi_get_str(vpiName, t2), "tb") == 0, "top via iterate");
    CHECK(vpi_scan(it) == NULL, "only one top module (iterator self-frees)");
    vpi_free_object(t2);

    /* --- child scopes --- */
    collect(vpiInternalScope, top, buf, sizeof buf);
    CHECK(strcmp(buf, "u_sub") == 0, "vpiInternalScope yields child instances");
    collect(vpiModule, top, buf, sizeof buf);
    CHECK(strcmp(buf, "u_sub") == 0, "vpiModule yields child instances");

    /* --- declared objects, by relation --- */
    collect(vpiNet, top, buf, sizeof buf);
    CHECK(strcmp(buf, "w") == 0, "vpiNet");
    collect(vpiReg, top, buf, sizeof buf);
    CHECK(strcmp(buf, "clk,data") == 0, "vpiReg");
    collect(vpiParameter, top, buf, sizeof buf);
    CHECK(strcmp(buf, "WIDTH") == 0, "vpiParameter (not vpiReg)");
    collect(vpiMemory, top, buf, sizeof buf);
    CHECK(strcmp(buf, "mem") == 0, "vpiMemory");

    /* --- the sub-module's scope --- */
    vpiHandle sub = vpi_handle_by_name("tb.u_sub", NULL);
    CHECK(sub != NULL, "an instance resolves by name");
    CHECK(vpi_get(vpiType, sub) == vpiModule, "an instance is a vpiModule, not a 1-bit signal");
    CHECK(strcmp(vpi_get_str(vpiDefName, sub), "sub") == 0, "vpiDefName");
    CHECK(strcmp(vpi_get_str(vpiFullName, sub), "tb.u_sub") == 0, "vpiFullName");

    /* Two live vpi_get_str results at once: the pool must not alias them. */
    const char *n1 = vpi_get_str(vpiName, sub);
    const char *n2 = vpi_get_str(vpiDefName, sub);
    CHECK(strcmp(n1, "u_sub") == 0 && strcmp(n2, "sub") == 0, "vpi_get_str results do not alias");

    vpiHandle parent = vpi_handle(vpiScope, sub);
    CHECK(parent && strcmp(vpi_get_str(vpiName, parent), "tb") == 0, "vpiScope of an instance is its parent");

    /* A sub-module's own objects, ports included (`clk` is a port of `sub`,
     * and has its own signal in the instance's scope). */
    collect(vpiReg, sub, buf, sizeof buf);
    CHECK(strcmp(buf, "clk,i,o") == 0, "a sub-module's own objects");

    /* --- values --- */
    s_vpi_value v;
    vpiHandle data = vpi_handle_by_name("tb.data", NULL);
    v.format = vpiHexStrVal;
    vpi_get_value(data, &v);
    CHECK(strcmp(v.value.str, "a5") == 0, "vpiHexStrVal through a VPI module");

    /* A packed-struct member is a part-select of its parent. */
    vpiHandle red = vpi_handle_by_name("tb.px.r", NULL);
    CHECK(red != NULL, "a packed-struct member resolves");
    CHECK(vpi_get(vpiSize, red) == 8, "member width");
    v.format = vpiHexStrVal;
    vpi_get_value(red, &v);
    CHECK(strcmp(v.value.str, "ff") == 0, "member read");
    /* ...and is writable in place, without disturbing its siblings. */
    v.format = vpiIntVal;
    v.value.integer = 0x11;
    vpi_put_value(red, &v, NULL, vpiNoDelay);

    /* --- memory words --- */
    vpiHandle mem = vpi_handle_by_name("tb.mem", NULL);
    CHECK(mem && vpi_get(vpiType, mem) == vpiMemory, "an unpacked array is a vpiMemory");
    CHECK(vpi_get(vpiSize, mem) == 4, "vpiSize of a memory is its word count");
    vpiHandle w1 = vpi_handle_by_index(mem, 1);
    CHECK(w1 != NULL, "vpi_handle_by_index");
    v.format = vpiIntVal;
    vpi_get_value(w1, &v);
    CHECK((unsigned)v.value.integer == 0xBEEFu, "memory word read");
    CHECK(vpi_handle_by_index(mem, 99) == NULL, "an out-of-range index is NULL");

    /* --- objects that have no value must say so --- */
    v.format = vpiIntVal;
    vpi_get_value(top, &v);
    CHECK(v.format == vpiSuppressVal, "a module has no value");

    vpi_printf("OM_ERRORS: %d\n", errors);
    return 0;
}

static void om_register(void) {
    s_vpi_systf_data d;
    memset(&d, 0, sizeof d);
    d.type = vpiSysTask;
    d.tfname = "$vpi_om_check";
    d.calltf = om_check;
    d.user_data = "cookie";
    vpi_register_systf(&d);
}

void (*vlog_startup_routines[])(void) = { om_register, 0 };
