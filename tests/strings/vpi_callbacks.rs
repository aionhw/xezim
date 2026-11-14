//! IEEE 1800-2017 §38.36 `vpi_register_cb`: every callback reason, with
//! `vpi_get_cb_info`, `vpi_remove_cb` and `vpi_control` (§38.14).
//!
//! Each test loads a VPI module through `--vpi-lib` and asserts the exact
//! order and times of the callbacks, interleaved with the design's own
//! `$display` output.

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("xezim_vpicb_{}_{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn shared_lib(dir: &Path, src: &str) -> PathBuf {
    let c = dir.join("cb.c");
    let so = dir.join("cb.so");
    std::fs::write(&c, src).unwrap();
    let include = Path::new(env!("CARGO_MANIFEST_DIR")).join("include");
    let ok = Command::new(std::env::var_os("CC").unwrap_or_else(|| "clang".into()))
        .args(["-shared", "-fPIC", "-Wall", "-I"])
        .arg(&include)
        .arg(&c)
        .arg("-o")
        .arg(&so)
        .status()
        .expect("failed to launch cc")
        .success();
    assert!(ok, "cc failed for {}", c.display());
    so
}

/// Run `sv` with the VPI module built from `c`; returns the combined output.
fn run(tag: &str, c: &str, sv: &str, extra: &[&str]) -> (String, bool) {
    let dir = scratch(tag);
    let so = shared_lib(&dir, c);
    std::fs::write(dir.join("top.sv"), sv).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .arg("--vpi-lib")
        .arg(&so)
        .args(["--no-cache", "-s", "top"])
        .args(extra)
        .arg("top.sv")
        .current_dir(&dir)
        .output()
        .expect("failed to run xezim");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
    (text, out.status.success())
}

/// The lines of `text` that start with one of `prefixes`, in order.
fn lines_with<'a>(text: &'a str, prefixes: &[&str]) -> Vec<&'a str> {
    text.lines()
        .map(|l| l.trim_end())
        .filter(|l| prefixes.iter().any(|p| l.starts_with(p)))
        .collect()
}

// ---------------------------------------------------------------------------
// Simulation-time callbacks
// ---------------------------------------------------------------------------

const TIME_C: &str = r#"
#include <stdio.h>
#include <string.h>
#include "vpi_user.h"

static const char *rname(int r) {
    switch (r) {
    case cbAtStartOfSimTime: return "AtStartOfSimTime";
    case cbNBASynch: return "NBASynch";
    case cbReadWriteSynch: return "ReadWriteSynch";
    case cbAtEndOfSimTime: return "AtEndOfSimTime";
    case cbReadOnlySynch: return "ReadOnlySynch";
    case cbNextSimTime: return "NextSimTime";
    case cbAfterDelay: return "AfterDelay";
    default: return "?";
    }
}

static vpiHandle reg_at(int reason, double t, int type, vpiHandle obj, int tag);

static int get_int(const char *name) {
    s_vpi_value v;
    v.format = vpiIntVal;
    vpi_get_value(vpi_handle_by_name((PLI_BYTE8 *)name, NULL), &v);
    return (int)v.value.integer;
}

static PLI_INT32 on_time(p_cb_data cb) {
    int tag = (int)(size_t)cb->user_data;
    if (cb->time->type == vpiScaledRealTime)
        vpi_printf("CB %s#%d t=%.3f b=%d\n", rname(cb->reason), tag, cb->time->real, get_int("top.b"));
    else
        vpi_printf("CB %s#%d t=%u b=%d\n", rname(cb->reason), tag, cb->time->low, get_int("top.b"));
    if (cb->reason == cbReadWriteSynch && tag == 2) {
        /* Writes are legal here and are seen in the same time step. */
        s_vpi_value v;
        v.format = vpiIntVal;
        v.value.integer = 1;
        vpi_put_value(vpi_handle_by_name("top.c", NULL), &v, NULL, vpiNoDelay);
        /* A same-step read-only callback, registered late, still fires. */
        reg_at(cbReadOnlySynch, 0, vpiSimTime, NULL, 3);
    }
    if (cb->reason == cbReadOnlySynch && tag == 2) {
        s_vpi_value v;
        v.format = vpiIntVal;
        v.value.integer = 0;
        vpi_put_value(vpi_handle_by_name("top.c", NULL), &v, NULL, vpiNoDelay);
        vpi_printf("RO put: chk=%d c=%d\n", vpi_chk_error(NULL), get_int("top.c"));
        vpi_printf("RO late RW: %s\n",
                   reg_at(cbReadWriteSynch, 0, vpiSimTime, NULL, 9) ? "registered" : "refused");
        vpi_chk_error(NULL);
    }
    return 0;
}

static vpiHandle reg_at(int reason, double t, int type, vpiHandle obj, int tag) {
    s_vpi_time tm;
    s_cb_data cb;
    memset(&cb, 0, sizeof cb);
    tm.type = type;
    tm.high = 0;
    tm.low = (PLI_UINT32)t;
    tm.real = t;
    cb.reason = reason;
    cb.cb_rtn = on_time;
    cb.obj = obj;
    cb.time = &tm;
    cb.user_data = (PLI_BYTE8 *)(size_t)tag;
    return vpi_register_cb(&cb);
}

static PLI_INT32 start(p_cb_data cb) {
    vpiHandle top = vpi_handle_by_name("top", NULL);
    (void)cb;
    /* Time 0: the current step, before any of its events. */
    reg_at(cbAtStartOfSimTime, 0, vpiSimTime, NULL, 0);
    reg_at(cbReadOnlySynch, 0, vpiSimTime, NULL, 0);
    reg_at(cbAtEndOfSimTime, 0, vpiSimTime, NULL, 0);
    reg_at(cbReadWriteSynch, 0, vpiSimTime, NULL, 0);
    reg_at(cbNBASynch, 0, vpiSimTime, NULL, 0);
    /* 2.5 ns in the top module's unit (1 ns; the tick is 1 ps). */
    reg_at(cbAfterDelay, 2.5, vpiScaledRealTime, top, 1);
    /* 5 ns: no HDL event at all. */
    reg_at(cbAtEndOfSimTime, 5000, vpiSimTime, NULL, 1);
    reg_at(cbAtStartOfSimTime, 5000, vpiSimTime, NULL, 1);
    reg_at(cbNextSimTime, 0, vpiSimTime, NULL, 1);
    /* 10 ns, registered in reverse order of firing. */
    reg_at(cbReadOnlySynch, 10000, vpiSimTime, NULL, 2);
    reg_at(cbAtEndOfSimTime, 10000, vpiSimTime, NULL, 2);
    reg_at(cbReadWriteSynch, 10, vpiScaledRealTime, top, 2);
    reg_at(cbNBASynch, 10000, vpiSimTime, NULL, 2);
    reg_at(cbAfterDelay, 10000, vpiSimTime, NULL, 2);
    reg_at(cbAtStartOfSimTime, 10000, vpiSimTime, NULL, 2);
    /* Refused: a time format that is no time, an absolute time without a
     * time, and a time already past. */
    vpi_printf("REG suppress=%s\n",
               reg_at(cbAtStartOfSimTime, 7, vpiSuppressTime, NULL, 8) ? "ok" : "NULL");
    vpi_printf("REG chk=%d\n", vpi_chk_error(NULL));
    {
        s_cb_data d;
        memset(&d, 0, sizeof d);
        d.reason = cbAtEndOfSimTime;
        d.cb_rtn = on_time;
        vpi_printf("REG notime=%s\n", vpi_register_cb(&d) ? "ok" : "NULL");
    }
    vpi_printf("REG unknown=%s\n", reg_at(999, 1, vpiSimTime, NULL, 8) ? "ok" : "NULL");
    vpi_chk_error(NULL);
    return 0;
}

static void boot(void) {
    s_cb_data cb;
    memset(&cb, 0, sizeof cb);
    cb.reason = cbStartOfSimulation;
    cb.cb_rtn = start;
    vpi_register_cb(&cb);
}
void (*vlog_startup_routines[])(void) = {boot, 0};
"#;

const TIME_SV: &str = r#"
`timescale 1ns/1ps
module top;
  reg [7:0] a = 0, b = 0;
  reg c = 0;
  initial begin
    $display("t=%0t active", $time);
    a <= 1;
    b <= 1;
    #10 $display("t=%0t active b=%0d", $time, b);
    b <= 2;
    #10 $display("t=%0t done c=%0d", $time, c);
  end
  always @(a) $display("t=%0t a=%0d", $time, a);
  always @(posedge c) $display("t=%0t posedge c", $time);
endmodule
"#;

/// Per time step: cbNextSimTime, cbAtStartOfSimTime, cbAfterDelay, (active
/// events), cbNBASynch, (NBA region and the processes it wakes),
/// cbReadWriteSynch, cbAtEndOfSimTime, cbReadOnlySynch. A time with no HDL
/// event still gets its callbacks, and a value deposited from
/// cbReadWriteSynch triggers `@(posedge)` within the same step.
#[test]
fn vpi_cb_simulation_time_order() {
    let (out, ok) = run("time", TIME_C, TIME_SV, &[]);
    assert!(ok, "{out}");
    let got = lines_with(&out, &["CB ", "t=", "REG ", "RO "]);
    let want = vec![
        "REG suppress=NULL",
        "REG chk=3",
        "REG notime=NULL",
        "REG unknown=NULL",
        "CB AtStartOfSimTime#0 t=0 b=0",
        "t=0 active",
        "CB NBASynch#0 t=0 b=0",
        "t=0 a=1",
        "CB ReadWriteSynch#0 t=0 b=1",
        "CB AtEndOfSimTime#0 t=0 b=1",
        "CB ReadOnlySynch#0 t=0 b=1",
        "CB NextSimTime#1 t=2500 b=1",
        "CB AfterDelay#1 t=2.500 b=1",
        "CB AtStartOfSimTime#1 t=5000 b=1",
        "CB AtEndOfSimTime#1 t=5000 b=1",
        "CB AtStartOfSimTime#2 t=10000 b=1",
        "CB AfterDelay#2 t=10000 b=1",
        "t=10000 active b=1",
        "CB NBASynch#2 t=10000 b=1",
        "CB ReadWriteSynch#2 t=10.000 b=2",
        "t=10000 posedge c",
        "CB AtEndOfSimTime#2 t=10000 b=2",
        "CB ReadOnlySynch#2 t=10000 b=2",
        "RO put: chk=3 c=1",
        "RO late RW: refused",
        "CB ReadOnlySynch#3 t=10000 b=2",
        "t=20000 done c=1",
    ];
    assert_eq!(got, want, "{out}");
}

// ---------------------------------------------------------------------------
// cbValueChange, vpi_get_cb_info, vpi_remove_cb
// ---------------------------------------------------------------------------

const VALUE_C: &str = r#"
#include <stdio.h>
#include <string.h>
#include "vpi_user.h"

static vpiHandle h_b2, h_self;
static int self_fires;

static void show_time(p_cb_data cb, char *buf) {
    switch (cb->time->type) {
    case vpiSimTime: sprintf(buf, "t=%u", cb->time->low); break;
    case vpiScaledRealTime: sprintf(buf, "t=%.1f", cb->time->real); break;
    case vpiSuppressTime: sprintf(buf, "t=suppressed"); break;
    default: sprintf(buf, "t=?%d", (int)cb->time->type);
    }
}

static PLI_INT32 on_change(p_cb_data cb) {
    char t[64];
    const char *tag = (const char *)cb->user_data;
    p_vpi_value v = cb->value;
    show_time(cb, t);
    switch (v->format) {
    case vpiScalarVal: vpi_printf("VC %s %s scalar=%d\n", tag, t, (int)v->value.scalar); break;
    case vpiIntVal: vpi_printf("VC %s %s int=%d index=%d\n", tag, t, (int)v->value.integer, (int)cb->index); break;
    case vpiRealVal: vpi_printf("VC %s %s real=%.2f\n", tag, t, v->value.real); break;
    case vpiVectorVal:
        vpi_printf("VC %s %s vec=%08x/%08x %08x/%08x\n", tag, t,
                   (unsigned)v->value.vector[1].aval, (unsigned)v->value.vector[1].bval,
                   (unsigned)v->value.vector[0].aval, (unsigned)v->value.vector[0].bval);
        break;
    case vpiDecStrVal: case vpiHexStrVal: case vpiBinStrVal:
        vpi_printf("VC %s %s str=%s\n", tag, t, v->value.str); break;
    case vpiSuppressVal: vpi_printf("VC %s %s suppressed\n", tag, t); break;
    default: vpi_printf("VC %s %s format=%d\n", tag, t, (int)v->format);
    }
    if (strcmp(tag, "B1") == 0 && h_b2) {
        /* Removes a later callback of the same batch: it must not run. */
        vpi_printf("remove B2: %d\n", vpi_remove_cb(h_b2));
        h_b2 = NULL;
    }
    if (strcmp(tag, "self") == 0 && ++self_fires == 2) {
        s_cb_data info;
        int rc;
        memset(&info, 0, sizeof info);
        rc = vpi_get_cb_info(h_self, &info);
        vpi_printf("info: %d reason=%d obj=%s fmt=%d ttype=%d ud=%s\n",
                   rc, (int)info.reason,
                   info.obj == cb->obj ? "same" : "other", (int)info.value->format,
                   (int)info.time->type, (const char *)info.user_data);
        vpi_printf("remove self: %d\n", vpi_remove_cb(h_self));
    }
    return 0;
}

static vpiHandle watch(const char *name, int format, int ttype, const char *tag) {
    s_vpi_time tm;
    s_vpi_value v;
    s_cb_data cb;
    memset(&cb, 0, sizeof cb);
    tm.type = ttype;
    v.format = format;
    cb.reason = cbValueChange;
    cb.cb_rtn = on_change;
    cb.obj = vpi_handle_by_name((PLI_BYTE8 *)name, NULL);
    cb.time = &tm;
    cb.value = &v;
    cb.user_data = (PLI_BYTE8 *)tag;
    if (!cb.obj) { vpi_printf("no object %s\n", name); return NULL; }
    return vpi_register_cb(&cb);
}

static PLI_INT32 start(p_cb_data cb) {
    vpiHandle h;
    (void)cb;
    h_self = watch("top.clk", vpiScalarVal, vpiSimTime, "self");
    vpi_printf("type: %d\n", (int)vpi_get(vpiType, h_self));
    watch("top.wide", vpiVectorVal, vpiSuppressTime, "wide");
    watch("top.wide", vpiSuppressVal, vpiSimTime, "wide-none");
    watch("top.n", vpiIntVal, vpiScaledRealTime, "B1");
    h_b2 = watch("top.n", vpiDecStrVal, vpiSimTime, "B2");
    /* Freeing the handle keeps the callback. */
    h = watch("top.r", vpiObjTypeVal, vpiSimTime, "r");
    vpi_printf("free: %d\n", vpi_free_object(h));
    watch("top.w", vpiHexStrVal, vpiSimTime, "w");
    watch("top.mem", vpiIntVal, vpiSimTime, "mem");
    watch("top.word.mid", vpiBinStrVal, vpiSimTime, "mid");
    /* Refused: no object, and a format no value can be rendered in. */
    vpi_printf("null obj: %s\n", watch("top.nosuch", vpiIntVal, vpiSimTime, "x") ? "ok" : "NULL");
    vpi_printf("bad format: %s\n", watch("top.n", 99, vpiSimTime, "x") ? "ok" : "NULL");
    vpi_chk_error(NULL);
    return 0;
}

static void boot(void) {
    s_cb_data cb;
    memset(&cb, 0, sizeof cb);
    cb.reason = cbStartOfSimulation;
    cb.cb_rtn = start;
    vpi_register_cb(&cb);
}
void (*vlog_startup_routines[])(void) = {boot, 0};
"#;

const VALUE_SV: &str = r#"
`timescale 1ns/1ns
module top;
  typedef struct packed { logic [3:0] hi; logic [7:0] mid; logic [3:0] lo; } w_t;
  reg clk = 0;
  reg [39:0] wide = 0;
  integer n = 0;
  real r = 0.0;
  reg [7:0] q = 0;
  wire [7:0] w = q + 8'd1;
  reg [7:0] mem [0:3];
  w_t word = 0;
  initial begin
    #1 clk = 1;
    #1 wide = 40'hz0_0000_00x1;
    #1 n = -5;
    $display("t=%0t n=%0d", $time, n);
    #1 r = 2.5;
    #1 q = 8'h7e;
    $display("t=%0t q=%h", $time, q);
    #1 mem[2] = 8'hab;
    #1 word = 16'h000f;
    #1 word = 16'h0a5f;
    #1 n = -5;
    #1 clk = 0;
    #1 clk = 1;
    #1 $display("t=%0t end", $time);
  end
endmodule
"#;

/// Every write path reports a change once (a blocking write before the next
/// statement runs, a continuous assignment after its settle), in the
/// registered value and time formats; a same-value write reports nothing.
/// A part-select reports only its own bits, a memory the word index. A
/// callback removed during its batch, or by itself, stops; freeing a handle
/// keeps the callback.
#[test]
fn vpi_cb_value_change_formats() {
    let (out, ok) = run("value", VALUE_C, VALUE_SV, &[]);
    assert!(ok, "{out}");
    let got = lines_with(
        &out,
        &[
            "VC ",
            "t=",
            "type:",
            "free:",
            "null obj",
            "bad format",
            "remove",
            "info:",
        ],
    );
    let want = vec![
        "type: 107",
        "free: 1",
        "null obj: NULL",
        "bad format: NULL",
        "VC self t=1 scalar=1",
        "VC wide t=suppressed vec=00000000/000000f0 000000f1/000000f0",
        "VC wide-none t=2 suppressed",
        "VC B1 t=3.0 int=-5 index=0",
        "remove B2: 1",
        "t=3 n=-5",
        "VC r t=4 real=2.50",
        "t=5 q=7e",
        "VC w t=5 str=7f",
        "VC mem t=6 int=171 index=2",
        "VC mid t=8 str=10100101",
        "VC self t=10 scalar=0",
        "info: 1 reason=1 obj=same fmt=5 ttype=2 ud=self",
        "remove self: 1",
        "t=12 end",
    ];
    assert_eq!(got, want, "{out}");
}

// ---------------------------------------------------------------------------
// cbForce / cbRelease / cbAssign / cbDeassign / cbDisable
// ---------------------------------------------------------------------------

const OVERRIDE_C: &str = r#"
#include <stdio.h>
#include <string.h>
#include "vpi_user.h"

static const char *rname(int r) {
    switch (r) {
    case cbForce: return "Force";
    case cbRelease: return "Release";
    case cbAssign: return "Assign";
    case cbDeassign: return "Deassign";
    case cbDisable: return "Disable";
    default: return "?";
    }
}

static PLI_INT32 on_event(p_cb_data cb) {
    const char *name = cb->obj ? vpi_get_str(vpiName, cb->obj) : "(null)";
    if (cb->value->format == vpiIntVal)
        vpi_printf("CB %s#%s obj=%s val=%d t=%u\n", rname(cb->reason),
                   (const char *)cb->user_data, name, (int)cb->value->value.integer, cb->time->low);
    else
        vpi_printf("CB %s#%s obj=%s t=%u\n", rname(cb->reason),
                   (const char *)cb->user_data, name, cb->time->low);
    return 0;
}

static vpiHandle on(int reason, const char *obj, const char *tag) {
    static s_vpi_time tm;
    static s_vpi_value v;
    s_cb_data cb;
    memset(&cb, 0, sizeof cb);
    tm.type = vpiSimTime;
    v.format = vpiIntVal;
    cb.reason = reason;
    cb.cb_rtn = on_event;
    cb.obj = obj ? vpi_handle_by_name((PLI_BYTE8 *)obj, NULL) : NULL;
    cb.time = &tm;
    cb.value = &v;
    cb.user_data = (PLI_BYTE8 *)tag;
    return vpi_register_cb(&cb);
}

static PLI_INT32 put_v(PLI_BYTE8 *ud) {
    s_vpi_value v;
    v.format = vpiIntVal;
    v.value.integer = 77;
    vpi_put_value(vpi_handle_by_name("top.v", NULL), &v, NULL,
                  ud ? vpiForceFlag : vpiReleaseFlag);
    return 0;
}

static PLI_INT32 arm_disable(PLI_BYTE8 *ud) {
    s_cb_data cb;
    static s_vpi_time tm;
    (void)ud;
    memset(&cb, 0, sizeof cb);
    tm.type = vpiSimTime;
    cb.reason = cbDisable;
    cb.cb_rtn = on_event;
    cb.obj = vpi_handle(vpiSysTfCall, NULL);
    cb.time = &tm;
    cb.user_data = "call";
    vpi_printf("arm disable: %s\n", vpi_register_cb(&cb) ? "ok" : "NULL");
    return 0;
}

static PLI_INT32 start(p_cb_data cb) {
    (void)cb;
    on(cbForce, "top.v", "v");
    on(cbForce, NULL, "all");
    on(cbRelease, NULL, "all");
    on(cbRelease, "top.nt", "nt");
    on(cbAssign, NULL, "all");
    on(cbDeassign, NULL, "all");
    /* A disable callback needs a $systf call (or a named scope) to watch. */
    vpi_printf("disable on a net: %s\n", on(cbDisable, "top.v", "x") ? "ok" : "NULL");
    vpi_chk_error(NULL);
    return 0;
}

static void boot(void) {
    s_cb_data cb;
    s_vpi_systf_data tf;
    memset(&cb, 0, sizeof cb);
    cb.reason = cbStartOfSimulation;
    cb.cb_rtn = start;
    vpi_register_cb(&cb);
    memset(&tf, 0, sizeof tf);
    tf.type = vpiSysTask;
    tf.tfname = "$vpi_force_v";
    tf.calltf = put_v;
    tf.user_data = "force";
    vpi_register_systf(&tf);
    tf.tfname = "$vpi_release_v";
    tf.user_data = NULL;
    vpi_register_systf(&tf);
    tf.tfname = "$arm_disable";
    tf.calltf = arm_disable;
    vpi_register_systf(&tf);
}
void (*vlog_startup_routines[])(void) = {boot, 0};
"#;

const OVERRIDE_SV: &str = r#"
`timescale 1ns/1ns
module top;
  reg [7:0] v = 1;
  reg [7:0] src = 3;
  wire [7:0] nt;
  assign nt = src;
  reg [7:0] pa = 0;
  initial begin
    #1 force v = 8'd42;
    $display("t=%0t v=%0d", $time, v);
    #1 release v;
    $display("t=%0t v=%0d", $time, v);
    #1 force nt = 8'd9;
    #1 release nt;
    #1 assign pa = src;
    #1 deassign pa;
    #1 $vpi_force_v;
    #1 $vpi_release_v;
    #1 $display("t=%0t v=%0d nt=%0d pa=%0d", $time, v, nt, pa);
  end
  initial begin : worker
    $arm_disable;
    #100 $display("worker was not disabled");
  end
  initial #10 disable worker;
endmodule
"#;

/// cbForce/cbAssign fire after the override took hold, cbRelease/cbDeassign
/// once the object is re-driven (a released net reports its driver's value),
/// for SystemVerilog statements and vpi_put_value alike; a NULL obj watches
/// every object. cbDisable on a `$systf` call fires when the named block
/// around the call is disabled.
#[test]
fn vpi_cb_force_release_assign_disable() {
    let (out, ok) = run("override", OVERRIDE_C, OVERRIDE_SV, &[]);
    assert!(ok, "{out}");
    let got = lines_with(&out, &["CB ", "t=", "arm ", "disable ", "worker"]);
    let want = vec![
        "disable on a net: NULL",
        "arm disable: ok",
        "CB Force#v obj=v val=42 t=1",
        "CB Force#all obj=v val=42 t=1",
        "t=1 v=42",
        "t=2 v=42",
        "CB Release#all obj=v val=42 t=2",
        "CB Force#all obj=nt val=9 t=3",
        "CB Release#all obj=nt val=3 t=4",
        "CB Release#nt obj=nt val=3 t=4",
        "CB Assign#all obj=pa val=3 t=5",
        "CB Deassign#all obj=pa val=3 t=6",
        "CB Force#v obj=v val=77 t=7",
        "CB Force#all obj=v val=77 t=7",
        "CB Release#all obj=v val=77 t=8",
        "t=9 v=77 nt=3 pa=3",
        "CB Disable#call obj=$arm_disable t=10",
    ];
    assert_eq!(got, want, "{out}");
}

// ---------------------------------------------------------------------------
// cbStmt
// ---------------------------------------------------------------------------

const STMT_C: &str = r#"
#include <stdio.h>
#include <string.h>
#include "vpi_user.h"

static int count;
static vpiHandle h_stmt;

static int get_int(const char *name) {
    s_vpi_value v;
    v.format = vpiIntVal;
    vpi_get_value(vpi_handle_by_name((PLI_BYTE8 *)name, NULL), &v);
    return (int)v.value.integer;
}

static PLI_INT32 on_stmt(p_cb_data cb) {
    count++;
    vpi_printf("STMT %d scope=%s t=%u k=%d\n", count, vpi_get_str(vpiFullName, cb->obj),
               cb->time->low, get_int("top.u.k"));
    if (count == 6) {
        vpi_printf("remove: %d\n", vpi_remove_cb(h_stmt));
    }
    return 0;
}

static PLI_INT32 start(p_cb_data cb) {
    static s_vpi_time tm;
    s_cb_data d;
    (void)cb;
    memset(&d, 0, sizeof d);
    tm.type = vpiSimTime;
    d.reason = cbStmt;
    d.cb_rtn = on_stmt;
    d.obj = vpi_handle_by_name("top.u", NULL);
    d.time = &tm;
    h_stmt = vpi_register_cb(&d);
    vpi_printf("stmt on top.u: %s\n", h_stmt ? "ok" : "NULL");
    /* No statement objects: a net is not a scope. */
    d.obj = vpi_handle_by_name("top.j", NULL);
    vpi_printf("stmt on a net: %s\n", vpi_register_cb(&d) ? "ok" : "NULL");
    vpi_chk_error(NULL);
    return 0;
}

static void boot(void) {
    s_cb_data cb;
    memset(&cb, 0, sizeof cb);
    cb.reason = cbStartOfSimulation;
    cb.cb_rtn = start;
    vpi_register_cb(&cb);
}
void (*vlog_startup_routines[])(void) = {boot, 0};
"#;

const STMT_SV: &str = r#"
`timescale 1ns/1ns
module sub;
  integer k = 0;
  initial begin
    k = 1;
    k = k + 1;
    if (k == 2) k = 5;
    $display("t=%0t sub k=%0d", $time, k);
    #1 k = 6;
    k = 7;
    $display("t=%0t sub k=%0d", $time, k);
  end
endmodule
module top;
  integer j = 0;
  sub u();
  initial begin
    j = 1;
    $display("t=%0t top j=%0d", $time, j);
  end
endmodule
"#;

/// cbStmt on a module instance runs before each statement a process of that
/// instance executes — not for other instances — and stops once removed.
#[test]
fn vpi_cb_stmt_scope() {
    let (out, ok) = run("stmt", STMT_C, STMT_SV, &[]);
    assert!(ok, "{out}");
    let got = lines_with(&out, &["STMT ", "t=", "stmt ", "remove"]);
    let want = vec![
        "stmt on top.u: ok",
        "stmt on a net: NULL",
        "STMT 1 scope=top.u t=0 k=0",
        "STMT 2 scope=top.u t=0 k=1",
        "STMT 3 scope=top.u t=0 k=2",
        "STMT 4 scope=top.u t=0 k=2",
        "STMT 5 scope=top.u t=0 k=5",
        "t=0 sub k=5",
        "STMT 6 scope=top.u t=0 k=5",
        "remove: 1",
        "t=0 top j=1",
        "t=1 sub k=7",
    ];
    assert_eq!(got, want, "{out}");
}

// ---------------------------------------------------------------------------
// Action callbacks and vpi_control
// ---------------------------------------------------------------------------

const ACTION_C: &str = r#"
#include <stdio.h>
#include <string.h>
#include "vpi_user.h"

static const char *rname(int r) {
    switch (r) {
    case cbEndOfCompile: return "EndOfCompile";
    case cbStartOfSimulation: return "StartOfSimulation";
    case cbEndOfSimulation: return "EndOfSimulation";
    case cbError: return "Error";
    case cbPLIError: return "PLIError";
    case cbTchkViolation: return "TchkViolation";
    case cbUnresolvedSystf: return "UnresolvedSystf";
    case cbEnterInteractive: return "EnterInteractive";
    case cbExitInteractive: return "ExitInteractive";
    case cbInteractiveScopeChange: return "InteractiveScopeChange";
    case cbStartOfSave: return "StartOfSave";
    case cbEndOfSave: return "EndOfSave";
    case cbStartOfRestart: return "StartOfRestart";
    case cbEndOfRestart: return "EndOfRestart";
    case cbStartOfReset: return "StartOfReset";
    case cbEndOfReset: return "EndOfReset";
    default: return "?";
    }
}

static PLI_INT32 late_task(PLI_BYTE8 *ud) {
    s_vpi_value v;
    vpiHandle args = vpi_iterate(vpiArgument, vpi_handle(vpiSysTfCall, NULL));
    vpiHandle a = vpi_scan(args);
    (void)ud;
    v.format = vpiIntVal;
    vpi_get_value(a, &v);
    vpi_free_object(args);
    vpi_printf("late_task(%d)\n", (int)v.value.integer);
    return 0;
}

static PLI_INT32 on_action(p_cb_data cb) {
    s_vpi_error_info e;
    vpi_printf("ACT %s t=%u", rname(cb->reason), cb->time->low);
    switch (cb->reason) {
    case cbError:
    case cbPLIError: {
        int level;
        memset(&e, 0, sizeof e);
        level = vpi_chk_error(&e);
        vpi_printf(" level=%d msg=%s", level, e.message);
        break;
    }
    case cbTchkViolation:
        vpi_printf(" obj=%s text=%s", cb->obj ? "set" : "NULL", cb->value->value.str);
        break;
    case cbUnresolvedSystf:
        vpi_printf(" name=%s", cb->value->value.str);
        if (strcmp(cb->value->value.str, "$late_task") == 0) {
            s_vpi_systf_data tf;
            memset(&tf, 0, sizeof tf);
            tf.type = vpiSysTask;
            tf.tfname = "$late_task";
            tf.calltf = late_task;
            vpi_register_systf(&tf);
        }
        break;
    case cbInteractiveScopeChange:
        vpi_printf(" scope=%s", vpi_get_str(vpiFullName, cb->obj));
        break;
    }
    vpi_printf("\n");
    return 0;
}

static vpiHandle on(int reason) {
    s_cb_data cb;
    memset(&cb, 0, sizeof cb);
    cb.reason = reason;
    cb.cb_rtn = on_action;
    return vpi_register_cb(&cb);
}

static PLI_INT32 bad_call(PLI_BYTE8 *ud) {
    s_vpi_value v;
    (void)ud;
    v.format = vpiIntVal;
    vpi_get_value(NULL, &v);
    /* The caller still sees the error after cbPLIError ran. */
    vpi_printf("after: chk=%d\n", (int)vpi_chk_error(NULL));
    return 0;
}

static PLI_INT32 start(p_cb_data cb) {
    int r;
    on_action(cb);
    for (r = cbError; r <= cbPLIError; r++)
        if (r != cbAssign && r != cbDeassign && r != cbDisable && r != cbStartOfSimulation)
            vpi_printf("reg %s: %s\n", rname(r), on(r) ? "ok" : "NULL");
    vpi_printf("reset: %d\n", (int)vpi_control(vpiReset, 0, 0, 0));
    vpi_chk_error(NULL);
    vpi_printf("scope: %d\n", (int)vpi_control(vpiSetInteractiveScope,
                                                vpi_handle_by_name("top", NULL)));
    return 0;
}

static void boot(void) {
    s_cb_data cb;
    s_vpi_systf_data tf;
    memset(&cb, 0, sizeof cb);
    cb.cb_rtn = on_action;
    cb.reason = cbEndOfSimulation;
    vpi_register_cb(&cb);
    cb.cb_rtn = start;
    cb.reason = cbStartOfSimulation;
    vpi_register_cb(&cb);
    cb.cb_rtn = on_action;
    cb.reason = cbEndOfCompile;
    vpi_register_cb(&cb);
    memset(&tf, 0, sizeof tf);
    tf.type = vpiSysTask;
    tf.tfname = "$bad_call";
    tf.calltf = bad_call;
    vpi_register_systf(&tf);
}
void (*vlog_startup_routines[])(void) = {boot, 0};
"#;

const ACTION_SV: &str = r#"
`timescale 1ns/1ns
module ff(input d, input clk);
  specify
    $setup(d, posedge clk, 5);
  endspecify
endmodule
module top;
  reg d = 0, clk = 0;
  ff u(.d(d), .clk(clk));
  initial begin
    #1 $error("boom %0d", 1);
    #1 $bad_call;
    #1 $late_task(7);
    $late_task(8);
    #1 $never_known;
    #4 d = 1;
    #2 clk = 1;
    #2 $display("t=%0t stopping", $time);
    $stop;
  end
endmodule
"#;

/// cbEndOfCompile precedes cbStartOfSimulation whatever the registration
/// order; cbError follows `$error` and a reported timing violation, with the
/// error readable through vpi_chk_error; cbPLIError follows a failing VPI
/// call without hiding the error from the caller; cbUnresolvedSystf can
/// register the unknown task, which then runs; `$stop` is reported as
/// cbEnterInteractive, then cbEndOfSimulation. The save/restart/reset and
/// exit-interactive reasons register but never fire.
#[test]
fn vpi_cb_actions_and_control() {
    let (out, ok) = run("action", ACTION_C, ACTION_SV, &[]);
    assert!(ok, "{out}");
    let got = lines_with(
        &out,
        &[
            "ACT ",
            "reg ",
            "reset",
            "scope",
            "late_task",
            "after",
            "t=",
            "** Error",
        ],
    );
    let want = vec![
        "ACT EndOfCompile t=0",
        "ACT StartOfSimulation t=0",
        "reg Error: ok",
        "reg TchkViolation: ok",
        "reg StartOfSave: ok",
        "reg EndOfSave: ok",
        "reg StartOfRestart: ok",
        "reg EndOfRestart: ok",
        "reg StartOfReset: ok",
        "reg EndOfReset: ok",
        "reg EnterInteractive: ok",
        "reg ExitInteractive: ok",
        "reg InteractiveScopeChange: ok",
        "reg UnresolvedSystf: ok",
        "reg PLIError: ok",
        "ACT PLIError t=0 level=3 msg=vpi_control(vpiReset): xezim cannot reset a running simulation",
        "reset: 0",
        "ACT InteractiveScopeChange t=0 scope=top",
        "scope: 1",
        "** Error: boom 1",
        "ACT Error t=1 level=3 msg=** Error: boom 1",
        "ACT PLIError t=2 level=3 msg=vpi_get_value: null handle",
        "after: chk=3",
        "ACT UnresolvedSystf t=3 name=$late_task",
        "late_task(7)",
        "late_task(8)",
        "ACT UnresolvedSystf t=4 name=$never_known",
        "** Error: $setup( d:8 ns, posedge clk:10 ns, 5 ns ) violation in top.u at time 10 ns (top.sv:5:5)",
        "ACT TchkViolation t=10 obj=NULL text=$setup( d:8 ns, posedge clk:10 ns, 5 ns ) violation in top.u",
        "ACT Error t=10 level=3 msg=** Error: $setup( d:8 ns, posedge clk:10 ns, 5 ns ) violation in top.u at time 10 ns (top.sv:5:5)",
        "t=12 stopping",
        "ACT EnterInteractive t=12",
        "ACT EndOfSimulation t=12",
    ];
    assert_eq!(got, want, "{out}");
}

// ---------------------------------------------------------------------------
// Callback lifecycle and cbSignal
// ---------------------------------------------------------------------------

const LIFE_C: &str = r#"
#include <signal.h>
#include <stdio.h>
#include <string.h>
#include "vpi_user.h"

static vpiHandle h_b, h_c;

static PLI_INT32 on_cb(p_cb_data cb);

static vpiHandle at(int reason, unsigned t, const char *tag) {
    s_vpi_time tm;
    s_cb_data cb;
    memset(&cb, 0, sizeof cb);
    tm.type = vpiSimTime;
    tm.high = 0;
    tm.low = t;
    cb.reason = reason;
    cb.cb_rtn = on_cb;
    cb.time = &tm;
    cb.user_data = (PLI_BYTE8 *)tag;
    return vpi_register_cb(&cb);
}

static PLI_INT32 on_cb(p_cb_data cb) {
    const char *tag = (const char *)cb->user_data;
    s_cb_data info;
    vpi_printf("CB %s t=%u index=%d\n", tag, cb->time->low, (int)cb->index);
    if (strcmp(tag, "A") == 0) {
        /* B is due in this same batch; removing it stops it. */
        vpi_printf("remove B: %d\n", vpi_remove_cb(h_b));
    } else if (strcmp(tag, "C") == 0) {
        /* A one-shot is still inspectable while it runs... */
        memset(&info, 0, sizeof info);
        vpi_printf("info C in C: %d\n", vpi_get_cb_info(h_c, &info));
    } else if (strcmp(tag, "D") == 0) {
        /* ...and gone once it has run; removing it just frees the handle. */
        vpi_printf("info C: %d\n", vpi_get_cb_info(h_c, &info));
        vpi_chk_error(NULL);
        vpi_printf("remove C: %d\n", vpi_remove_cb(h_c));
        vpi_printf("remove NULL: %d\n", vpi_remove_cb(NULL));
        vpi_chk_error(NULL);
    } else if (strcmp(tag, "E") == 0) {
        raise(SIGINT);
    }
    return 0;
}

static PLI_INT32 start(p_cb_data cb) {
    s_cb_data d;
    (void)cb;
    at(cbAfterDelay, 1, "A");
    h_b = at(cbAfterDelay, 1, "B");
    h_c = at(cbAfterDelay, 2, "C");
    at(cbAtStartOfSimTime, 3, "D");
    at(cbAfterDelay, 5, "E");
    memset(&d, 0, sizeof d);
    d.cb_rtn = on_cb;
    d.reason = cbSignal;
    d.user_data = "signal";
    vpi_register_cb(&d);
    d.reason = cbEndOfSimulation;
    d.user_data = "end";
    vpi_register_cb(&d);
    return 0;
}

static void boot(void) {
    s_cb_data cb;
    memset(&cb, 0, sizeof cb);
    cb.reason = cbStartOfSimulation;
    cb.cb_rtn = start;
    vpi_register_cb(&cb);
}
void (*vlog_startup_routines[])(void) = {boot, 0};
"#;

const LIFE_SV: &str = r#"
`timescale 1ns/1ns
module top;
  reg clk = 0;
  always #1 clk = ~clk;
endmodule
"#;

/// A callback removed by an earlier one of its batch does not run; a
/// one-shot can be inspected while it runs and is gone afterwards, when
/// removing it only frees the handle. SIGINT is reported through cbSignal
/// (index = the signal number) before the run ends.
#[test]
fn vpi_cb_lifecycle_and_signal() {
    let (out, _ok) = run("life", LIFE_C, LIFE_SV, &[]);
    let got = lines_with(&out, &["CB ", "remove", "info"]);
    let want = vec![
        "CB A t=1 index=0",
        "remove B: 1",
        "CB C t=2 index=0",
        "info C in C: 1",
        "CB D t=3 index=0",
        "info C: 0",
        "remove C: 1",
        "remove NULL: 0",
        "CB E t=5 index=0",
        "CB signal t=5 index=2",
        "CB end t=5 index=0",
    ];
    assert_eq!(got, want, "{out}");
}

/// §20.2 / §38.4: a callback's finish prevents delivery of the edge it drove.
#[test]
fn next_twelve_vpi_finish_stops_awakened_processes() {
    let (text, success) = run(
        "finish_edge_371",
        include_str!("../issue_cases/next_issue371_2.c"),
        include_str!("../issue_cases/next_issue371_1.sv"),
        &[],
    );
    assert!(success, "{text}");
    assert!(text.contains("PASS n=2"), "{text}");
    assert!(!text.contains("FAIL"), "{text}");
}

/// §9.2.3: end callbacks observe writes from every final procedure.
#[test]
fn next_rest_issue373_final_before_end_callback() {
    let (text, success) = run(
        "final_before_end_373",
        include_str!("../issue_cases/rest_issue373_1.c"),
        include_str!("../issue_cases/rest_issue373_0.sv"),
        &[],
    );
    assert!(success, "{text}");
    assert!(!text.contains("FAIL"), "{text}");
    assert!(
        text.contains("cbEndOfSimulation: top.final_ran = 1"),
        "{text}"
    );
    assert!(
        text.find("final block").unwrap() < text.find("cbEndOfSimulation:").unwrap(),
        "{text}"
    );
}

// ---------------------------------------------------------------------------
// A deposit from inside a routine (#811)
// ---------------------------------------------------------------------------

const NESTED_C: &str = r#"
#include <stdio.h>
#include "vpi_user.h"

static int depth;
static vpiHandle clk;

static PLI_INT32 on_change(p_cb_data cb) {
    vpi_printf("V|change t=%u val=%d nested=%d\n", cb->time->low,
               (int)cb->value->value.integer, depth);
    return 0;
}

static PLI_INT32 on_delay(p_cb_data cb) {
    s_vpi_value v;
    s_vpi_time now;
    depth++;
    v.format = vpiIntVal;
    v.value.integer = (int)(PLI_INT32)(cb->user_data != NULL);
    vpi_put_value(clk, &v, NULL, vpiNoDelay);
    now.type = vpiSimTime;
    vpi_get_time(NULL, &now);
    vpi_printf("V|put t=%u\n", now.low);
    depth--;
    return 0;
}

static void after(PLI_UINT32 t, int high) {
    static s_vpi_time dt;
    s_cb_data d = {0};
    dt.type = vpiSimTime;
    dt.high = 0;
    dt.low = t;
    d.reason = cbAfterDelay;
    d.cb_rtn = on_delay;
    d.time = &dt;
    d.user_data = high ? (PLI_BYTE8 *)"1" : NULL;
    vpi_register_cb(&d);
}

static PLI_INT32 start(p_cb_data cb) {
    static s_vpi_time t;
    static s_vpi_value v;
    s_cb_data d = {0};
    (void)cb;
    clk = vpi_handle_by_name("top.clk", NULL);
    t.type = vpiSimTime;
    v.format = vpiIntVal;
    d.reason = cbValueChange;
    d.cb_rtn = on_change;
    d.obj = clk;
    d.time = &t;
    d.value = &v;
    vpi_register_cb(&d);
    after(5, 1);
    after(10, 0);
    return 0;
}

static void reg(void) {
    s_cb_data d = {0};
    d.reason = cbStartOfSimulation;
    d.cb_rtn = start;
    vpi_register_cb(&d);
}

void (*vlog_startup_routines[])(void) = {reg, 0};
"#;

const NESTED_SV: &str = r#"
module top;
  logic clk = 0;
  always @(posedge clk) $display("V|posedge t=%0t", $time);
  initial #20 $finish;
endmodule
"#;

/// §38.36.1: a value change made by `vpi_put_value` inside a routine is
/// reported after that routine returns, in the same time slot, not by a call
/// into the value-change routine while it still runs. cocotb 1.x aborts on
/// such a nested call ("calling up again") when a test drives a clock from
/// Python and waits on its edge (#811).
#[test]
fn vpi_cb_value_change_of_a_routine_deposit_is_not_nested() {
    let (out, ok) = run("nested", NESTED_C, NESTED_SV, &[]);
    assert!(ok, "{out}");
    let got = lines_with(&out, &["V|"]);
    let want = vec![
        "V|put t=5",
        "V|change t=5 val=1 nested=0",
        "V|posedge t=5",
        "V|put t=10",
        "V|change t=10 val=0 nested=0",
    ];
    assert_eq!(got, want, "{out}");
}

/// IEEE 1800-2023 §9.6.2 / §38.36: a callback registered by a killed
/// child retains that process's identity when later children are spawned.
/// One FORK_CB per disable follows xezim's `vpi_cb.rs` rule: a cbDisable
/// on a system-task call belongs to the calling process, not an LRM count.
#[test]
fn finished_fork_children_keep_vpi_callback_identity() {
    let (out, ok) = run(
        "finished_children",
        include_str!("../dpi/finished_process_callbacks.c"),
        include_str!("../dpi/finished_process_callbacks.sv"),
        &[],
    );
    assert!(ok, "{out}");
    assert_eq!(
        lines_with(&out, &["FORK_CB ", "VPI_FORK_PASS"]),
        [
            "FORK_CB tag=1 t=2 type=57",
            "FORK_CB tag=2 t=5 type=57",
            "FORK_CB tag=3 t=8 type=57",
            "FORK_CB tag=4 t=11 type=57",
            "FORK_CB tag=5 t=14 type=57",
            "VPI_FORK_PASS t=15",
        ],
        "{out}"
    );
    // vpiInitial is still the retained static process handle's type (§37).
    assert!(
        out.lines().any(
            |line| line.starts_with("FORK_CB_END tags=5 ") && line.ends_with("initial_type=24")
        ),
        "{out}"
    );
}
