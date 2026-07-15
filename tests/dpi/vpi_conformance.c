/* Regression coverage for the IEEE 1800-2017 clause 38 (VPI) audit.
 *
 * Each check below corresponds to a defect that shipped:
 *
 *   1. vpi_handle_by_name spun forever on any dotted name that did not
 *      resolve on the first try — reachable straight from
 *      uvm_hdl_check_path. A regression here HANGS rather than fails, so
 *      the Rust harness runs this binary under a wall-clock timeout.
 *   2. vpi_get_value handled only vpiIntVal and vpiRealVal and silently
 *      ignored everything else, including the vpiVectorVal that UVM's HDL
 *      backdoor reads with; and it never signalled failure.
 *   3. vpi_put_value with vpiVectorVal dropped the upper word of a 33..64
 *      bit signal, masked X/Z away, and capped the wide path at 128 bits.
 *   4. vpi_get(vpiType) answered from the signal's CURRENT VALUE, and
 *      returned codes present in no header.
 *   5. vpi_get_vlog_info hardcoded a version string.
 */
#include <stdio.h>
#include <string.h>
#include <stdint.h>
#include "vpi_user.h"
#include "svdpi.h"

static int errors = 0;
static vpiHandle g_start_sim_h = NULL;
static vpiHandle g_end_sim_h = NULL;

#define CHECK(cond, msg)                                                   \
    do {                                                                   \
        if (!(cond)) {                                                     \
            printf("FAIL: %s\n", (msg));                                   \
            errors++;                                                      \
        }                                                                  \
    } while (0)

/* --- 1. name resolution, including the shapes that used to hang ------ */
int vc_names(void) {
    vpiHandle h;

    h = vpi_handle_by_name("tb.sig32", NULL);
    CHECK(h != NULL, "full hierarchical name must resolve");
    vpi_free_object(h);

    h = vpi_handle_by_name("sig32", NULL);
    CHECK(h != NULL, "leaf name must resolve");
    vpi_free_object(h);

    /* Walks suffixes: tb.sig32 is found after stripping "a." and "b." */
    h = vpi_handle_by_name("a.b.tb.sig32", NULL);
    CHECK(h != NULL, "a longer prefix must be stripped one segment at a time");
    vpi_free_object(h);

    /* These two used to spin forever. */
    h = vpi_handle_by_name("top.no.such", NULL);
    CHECK(h == NULL, "an unresolvable dotted name must return NULL, not hang");

    h = vpi_handle_by_name("nosuchsignal", NULL);
    CHECK(h == NULL, "an unresolvable leaf name must return NULL");

    /* vpi_handle models no relationships and must say so, not fail to link. */
    h = vpi_handle_by_name("tb.sig32", NULL);
    CHECK(vpi_handle(vpiNet, h) == NULL, "vpi_handle returns NULL");
    vpi_free_object(h);
    return 0;
}

/* --- 2. every vpi_get_value format, and the failure channel ---------- */
int vc_get_value(void) {
    vpiHandle h = vpi_handle_by_name("tb.sig32", NULL); /* 32'h1234ABCD */
    s_vpi_value v;

    v.format = vpiIntVal;
    vpi_get_value(h, &v);
    CHECK(v.format == vpiIntVal && (unsigned)v.value.integer == 0x1234ABCDu, "vpiIntVal");

    v.format = vpiHexStrVal;
    vpi_get_value(h, &v);
    CHECK(v.format == vpiHexStrVal && strcmp(v.value.str, "1234abcd") == 0, "vpiHexStrVal");

    v.format = vpiBinStrVal;
    vpi_get_value(h, &v);
    CHECK(strncmp(v.value.str, "00010010", 8) == 0, "vpiBinStrVal");

    v.format = vpiDecStrVal;
    vpi_get_value(h, &v);
    CHECK(strcmp(v.value.str, "305441741") == 0, "vpiDecStrVal");

    v.format = vpiOctStrVal;
    vpi_get_value(h, &v);
    CHECK(strcmp(v.value.str, "02215125715") == 0, "vpiOctStrVal");

    /* vpiObjTypeVal: the simulator picks, and reports what it picked. */
    v.format = vpiObjTypeVal;
    vpi_get_value(h, &v);
    CHECK(v.format == vpiIntVal, "vpiObjTypeVal on a 32-bit signal picks vpiIntVal");

    /* An unsupported format must set vpiSuppressVal, not leave the union
     * untouched while claiming nothing went wrong. */
    v.format = vpiStrengthVal;
    vpi_get_value(h, &v);
    CHECK(v.format == vpiSuppressVal, "an unsupported format reports vpiSuppressVal");
    vpi_free_object(h);

    /* A NULL handle likewise. */
    v.format = vpiIntVal;
    vpi_get_value(NULL, &v);
    CHECK(v.format == vpiSuppressVal, "a NULL handle reports vpiSuppressVal");

    /* vpiVectorVal points at SIMULATOR-owned storage. The caller's own
     * pointer must not be assumed to have been filled. */
    h = vpi_handle_by_name("tb.wide", NULL); /* 128'h1122..FF00 */
    s_vpi_vecval poison[4];
    for (int i = 0; i < 4; i++) { poison[i].aval = 0xDEADBEEF; poison[i].bval = 0; }
    v.format = vpiVectorVal;
    v.value.vector = poison;
    vpi_get_value(h, &v);
    CHECK(v.format == vpiVectorVal, "vpiVectorVal is supported");
    CHECK(v.value.vector != poison, "the simulator supplies the vector buffer");
    CHECK((unsigned)v.value.vector[0].aval == 0xDDEEFF00u, "vector word 0");
    CHECK((unsigned)v.value.vector[1].aval == 0x99AABBCCu, "vector word 1");
    CHECK((unsigned)v.value.vector[3].aval == 0x11223344u, "vector word 3 (>64 bits)");

    /* A 4-state read must carry X/Z out in bval. tb.xz = 8'b1010_xzxz */
    vpi_free_object(h);
    h = vpi_handle_by_name("tb.xz", NULL);
    v.format = vpiVectorVal;
    vpi_get_value(h, &v);
    /* bit0=z bit1=x bit2=z bit3=x -> aval 0b1010_1010, bval 0b0000_1111 */
    CHECK((v.value.vector[0].aval & 0xFF) == 0xAA, "4-state read aval");
    CHECK((v.value.vector[0].bval & 0xFF) == 0x0F, "4-state read bval");
    vpi_free_object(h);

    /* Scalar. */
    h = vpi_handle_by_name("tb.one_bit", NULL);
    v.format = vpiScalarVal;
    vpi_get_value(h, &v);
    CHECK(v.value.scalar == vpi1, "vpiScalarVal");
    vpi_free_object(h);

    /* vpiTimeVal (clause 37): the s_vpi_time record must describe the VALUE
     * being read, not the simulation clock. tb.t42 holds 42 and is read at
     * sim time 1; high/low AND .real must all report 42. The old code set
     * .real = current_time, so it read back 1 — a self-inconsistent record. */
    h = vpi_handle_by_name("tb.t42", NULL);
    v.format = vpiTimeVal;
    vpi_get_value(h, &v);
    CHECK(v.format == vpiTimeVal, "vpiTimeVal is supported");
    CHECK(v.value.time->low == 42u, "vpiTimeVal low word is the value (42)");
    CHECK(v.value.time->high == 0u, "vpiTimeVal high word is 0");
    CHECK(v.value.time->real == 42.0, "vpiTimeVal .real describes the value, not the clock");
    vpi_free_object(h);
    return 0;
}

/* --- 3. vpi_put_value keeps every bit, including X and Z ------------- */
int vc_put_wide(void) {
    vpiHandle h = vpi_handle_by_name("tb.w64", NULL);
    s_vpi_vecval v[2];
    v[0].aval = (PLI_INT32)0xAAAABBBB; v[0].bval = 0;
    v[1].aval = (PLI_INT32)0xCCCCDDDD; v[1].bval = 0;  /* the upper word */
    s_vpi_value vs;
    vs.format = vpiVectorVal;
    vs.value.vector = v;
    vpi_put_value(h, &vs, NULL, vpiNoDelay);
    vpi_free_object(h);
    return 0;
}

int vc_put_xz(void) {
    /* aval=1,bval=1 -> X ; aval=0,bval=1 -> Z (IEEE 1800-2017 38.10.1) */
    vpiHandle h = vpi_handle_by_name("tb.put_x", NULL);
    s_vpi_vecval v[1];
    v[0].aval = (PLI_INT32)0xFF; v[0].bval = (PLI_INT32)0xF0;
    s_vpi_value vs; vs.format = vpiVectorVal; vs.value.vector = v;
    vpi_put_value(h, &vs, NULL, vpiNoDelay);
    vpi_free_object(h);

    h = vpi_handle_by_name("tb.put_z", NULL);
    v[0].aval = (PLI_INT32)0x0F; v[0].bval = (PLI_INT32)0xF0;
    vs.format = vpiVectorVal; vs.value.vector = v;
    vpi_put_value(h, &vs, NULL, vpiNoDelay);
    vpi_free_object(h);

    /* An undecodable format must write NOTHING. tb.untouched stays 8'hA5. */
    h = vpi_handle_by_name("tb.untouched", NULL);
    vs.format = vpiStrengthVal;
    vpi_put_value(h, &vs, NULL, vpiNoDelay);
    vpi_free_object(h);
    return 0;
}

/* --- 4. vpi_get is type-directed, not value-directed ----------------- */
int vc_get_props(void) {
    struct { const char *name; int type; int size; } want[] = {
        { "tb.sig32",   vpiReg,         32  },  /* logic  */
        { "tb.wide",    vpiBitVar,      128 },  /* bit    */
        { "tb.w64",     vpiReg,         64  },  /* logic  */
        { "tb.an_int",  vpiIntVar,      32  },  /* int    */
        { "tb.a_long",  vpiLongIntVar,  64  },  /* longint*/
        { "tb.a_real",  vpiRealVar,     64  },  /* real   */
        { "tb.a_net",   vpiNet,         8   },  /* wire   */
    };
    for (unsigned i = 0; i < sizeof(want) / sizeof(want[0]); i++) {
        vpiHandle h = vpi_handle_by_name((char *)want[i].name, NULL);
        if (!h) { printf("FAIL: no handle for %s\n", want[i].name); errors++; continue; }
        int ty = vpi_get(vpiType, h);
        int sz = vpi_get(vpiSize, h);
        if (ty != want[i].type) {
            printf("FAIL: vpiType(%s) = %d, want %d\n", want[i].name, ty, want[i].type);
            errors++;
        }
        if (sz != want[i].size) {
            printf("FAIL: vpiSize(%s) = %d, want %d\n", want[i].name, sz, want[i].size);
            errors++;
        }
        vpi_free_object(h);
    }

    /* tb.xz currently holds X/Z. Its TYPE is still vpiReg — the answer must
     * not depend on the value. */
    vpiHandle h = vpi_handle_by_name("tb.xz", NULL);
    CHECK(vpi_get(vpiType, h) == vpiReg, "vpiType must come from the declaration, not the value");
    CHECK(vpi_get(vpiSigned, h) == 0, "vpiSigned");
    CHECK(vpi_get(vpiVector, h) == 1, "vpiVector");
    CHECK(vpi_get(9999, h) == vpiUndefined, "an unmodelled property is vpiUndefined");
    vpi_free_object(h);

    h = vpi_handle_by_name("tb.one_bit", NULL);
    CHECK(vpi_get(vpiScalar, h) == 1, "vpiScalar");
    vpi_free_object(h);

    h = vpi_handle_by_name("tb.a_long", NULL);
    CHECK(vpi_get(vpiSigned, h) == 1, "longint is signed");
    vpi_free_object(h);
    return 0;
}

/* --- 5. vpi_get_vlog_info reports the real version ------------------- */
/* Prints it rather than asserting a literal, so the Rust harness can
 * compare against CARGO_PKG_VERSION and the check never goes stale. */
int vc_vlog_info(void) {
    s_vpi_vlog_info info;
    memset(&info, 0, sizeof info);
    int rc = vpi_get_vlog_info(&info);
    CHECK(rc == 1, "vpi_get_vlog_info returns 1");
    CHECK(info.product && strcmp(info.product, "xezim") == 0, "product is xezim");
    printf("VLOG_VERSION: %s\n", info.version ? info.version : "(null)");
    return 0;
}

/* --- 6. vpi_get_time reports current simulation time ----------------- */
int vc_get_time(int expect_tick) {
    s_vpi_time t;
    memset(&t, 0, sizeof t);

    t.type = vpiSimTime;
    vpi_get_time(NULL, &t);
    {
        uint64_t got = ((uint64_t)t.high << 32) | (uint64_t)t.low;
        CHECK(got == (uint64_t)expect_tick, "vpi_get_time(vpiSimTime)");
    }

    t.type = vpiScaledRealTime;
    t.high = 0;
    t.low = 0;
    t.real = -1.0;
    vpi_get_time(NULL, &t);
    CHECK(t.type == vpiScaledRealTime, "vpi_get_time preserves vpiScaledRealTime type");
    CHECK(t.real == (double)expect_tick, "vpi_get_time(vpiScaledRealTime)");

    return 0;
}

/* --- 7. vpi_get_cb_info reports callback registration data ----------- */
static PLI_INT32 vc_info_cb(p_cb_data cb_data_p) {
    (void)cb_data_p;
    return 0;
}

static PLI_INT32 vc_start_sim_cb(p_cb_data cb_data_p) {
    (void)cb_data_p;
    return 0;
}

static PLI_INT32 vc_end_sim_cb(p_cb_data cb_data_p) {
    (void)cb_data_p;
    return 0;
}

int vc_get_cb_info(void) {
    vpiHandle h = vpi_handle_by_name("tb.sig32", NULL);
    CHECK(h != NULL, "vpi_get_cb_info setup handle");
    if (h == NULL) {
        return 0;
    }

    int cookie = 0x51A7;
    s_vpi_value value;
    memset(&value, 0, sizeof value);
    value.format = vpiHexStrVal;

    s_cb_data cb;
    memset(&cb, 0, sizeof cb);
    cb.reason = cbValueChange;
    cb.cb_rtn = vc_info_cb;
    cb.obj = h;
    cb.value = &value;
    cb.user_data = (PLI_BYTE8 *)&cookie;

    vpiHandle cbh = vpi_register_cb(&cb);
    CHECK(cbh != NULL, "vpi_register_cb for vpi_get_cb_info");
    if (cbh == NULL) {
        vpi_free_object(h);
        return 0;
    }

    s_cb_data out;
    memset(&out, 0, sizeof out);
    CHECK(vpi_get_cb_info(cbh, &out) == 1, "vpi_get_cb_info returns 1");
    CHECK(out.reason == cbValueChange, "vpi_get_cb_info reason");
    CHECK(out.cb_rtn == vc_info_cb, "vpi_get_cb_info cb_rtn");
    CHECK(out.obj == h, "vpi_get_cb_info obj");
    CHECK(out.user_data == (PLI_BYTE8 *)&cookie, "vpi_get_cb_info user_data");
    CHECK(out.value != NULL && out.value->format == vpiHexStrVal,
          "vpi_get_cb_info value format");

    CHECK(vpi_remove_cb(cbh) == 1, "vpi_remove_cb after vpi_get_cb_info");
    vpi_free_object(h);
    return 0;
}

int vc_register_sim_phase_cbs(void) {
    s_cb_data cb;
    memset(&cb, 0, sizeof cb);

    cb.reason = cbStartOfSimulation;
    cb.cb_rtn = vc_start_sim_cb;
    g_start_sim_h = vpi_register_cb(&cb);
    CHECK(g_start_sim_h != NULL, "register cbStartOfSimulation");

    cb.reason = cbEndOfSimulation;
    cb.cb_rtn = vc_end_sim_cb;
    g_end_sim_h = vpi_register_cb(&cb);
    CHECK(g_end_sim_h != NULL, "register cbEndOfSimulation");

    return 0;
}

int vc_check_sim_phase_cb_info(void) {
    s_cb_data out;
    memset(&out, 0, sizeof out);

    CHECK(g_start_sim_h != NULL, "cbStartOfSimulation handle available");
    if (g_start_sim_h != NULL) {
        CHECK(vpi_get_cb_info(g_start_sim_h, &out) == 1, "get_cb_info start-sim");
        CHECK(out.reason == cbStartOfSimulation, "start-sim reason");
        CHECK(out.cb_rtn == vc_start_sim_cb, "start-sim cb_rtn");
        CHECK(vpi_remove_cb(g_start_sim_h) == 1, "remove start-sim callback");
        g_start_sim_h = NULL;
    }

    memset(&out, 0, sizeof out);
    CHECK(g_end_sim_h != NULL, "cbEndOfSimulation handle available");
    if (g_end_sim_h != NULL) {
        CHECK(vpi_get_cb_info(g_end_sim_h, &out) == 1, "get_cb_info end-sim");
        CHECK(out.reason == cbEndOfSimulation, "end-sim reason");
        CHECK(out.cb_rtn == vc_end_sim_cb, "end-sim cb_rtn");
        CHECK(vpi_remove_cb(g_end_sim_h) == 1, "remove end-sim callback");
        g_end_sim_h = NULL;
    }

    return 0;
}

int vc_errors(void) { return errors; }
