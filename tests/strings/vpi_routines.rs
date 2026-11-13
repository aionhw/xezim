//! The VPI routines of IEEE 1800-2017 clause 38 that xezim lacked: per-call
//! user data and `vpi_get_systf_info`, `vpi_handle_by_multi_index`,
//! `vpi_handle_multi`, `vpi_get_value_array` / `vpi_put_value_array`,
//! `vpi_get_delays` / `vpi_put_delays`, `vpi_get_data` / `vpi_put_data`, and
//! the value formats vpiStrengthVal, vpiShortIntVal, vpiLongIntVal,
//! vpiShortRealVal and the raw formats. Each test is a C library plus a
//! design run through the CLI; the C side prints `CHK|...` lines and `FAIL:`
//! for any check that does not hold.

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("xezim_{}_{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn shared_lib(dir: &Path, stem: &str, src: &str) -> PathBuf {
    let c = dir.join(format!("{stem}.c"));
    let so = dir.join(format!("{stem}.so"));
    std::fs::write(&c, src).unwrap();
    let include = Path::new(env!("CARGO_MANIFEST_DIR")).join("include");
    let ok = Command::new(std::env::var_os("CC").unwrap_or_else(|| "clang".into()))
        .args(["-shared", "-fPIC", "-Wall", "-Werror", "-I"])
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

/// Run `top.sv` with `lib` loaded as a VPI module (`--vpi-lib`) or a DPI
/// library, under a timeout, with `env` set.
fn run(dir: &Path, flag: &str, lib: &Path, sv: &str, env: &[(&str, &str)]) -> String {
    run_status(dir, flag, lib, sv, env, true)
}

/// `run`, optionally accepting a failing exit status (a design whose timing
/// checks report violations ends with one).
fn run_status(
    dir: &Path,
    flag: &str,
    lib: &Path,
    sv: &str,
    env: &[(&str, &str)],
    must_succeed: bool,
) -> String {
    use std::io::Read;
    std::fs::write(dir.join("top.sv"), sv).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_xezim"));
    cmd.arg(flag)
        .arg(lib)
        .args(["--no-cache", "-s", "top", "top.sv"])
        .current_dir(dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().expect("failed to run xezim");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    let status = loop {
        if let Some(s) = child.try_wait().expect("try_wait") {
            break s;
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("xezim did not finish within 120s in {}", dir.display());
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    let mut text = String::new();
    if let Some(mut o) = child.stdout.take() {
        let _ = o.read_to_string(&mut text);
    }
    if let Some(mut e) = child.stderr.take() {
        let _ = e.read_to_string(&mut text);
    }
    assert!(!must_succeed || status.success(), "xezim failed:\n{text}");
    assert!(!text.contains("FAIL:"), "C-side check failed:\n{text}");
    text
}

fn expect(text: &str, lines: &[&str]) {
    for line in lines {
        assert!(text.contains(line), "missing `{line}`:\n{text}");
    }
}

// ---------------------------------------------------------------------------
// vpi_put_userdata / vpi_get_userdata, vpi_get_systf_info, vpiUserSystf,
// compiletf once per call instance, vpi_get_data / vpi_put_data.
// ---------------------------------------------------------------------------

const SYSTF_C: &str = r#"
#include <stdlib.h>
#include <string.h>
#include "vpi_user.h"

#define CHECK(c, m) do { if (!(c)) vpi_printf("FAIL: %s\n", (m)); } while (0)

static int compiles = 0;
static vpiHandle tick_reg = NULL;

static PLI_INT32 tick_compile(PLI_BYTE8 *ud) {
    vpiHandle call = vpi_handle(vpiSysTfCall, NULL);
    CHECK(vpi_get_userdata(call) == NULL, "a new call instance has no user data");
    int *count = malloc(sizeof *count);
    *count = 0;
    CHECK(vpi_put_userdata(call, count) == 1, "vpi_put_userdata returns 1");
    compiles++;
    vpi_free_object(call);
    return 0;
}

static PLI_INT32 tick_call(PLI_BYTE8 *ud) {
    CHECK(ud == (PLI_BYTE8 *)0x1234, "calltf gets the registered user_data");
    vpiHandle call = vpi_handle(vpiSysTfCall, NULL);
    int *count = vpi_get_userdata(call);
    CHECK(count != NULL, "the call instance keeps its user data");
    vpiHandle it = vpi_iterate(vpiArgument, call);
    vpiHandle arg = vpi_scan(it);
    s_vpi_value v;
    v.format = vpiStringVal;
    vpi_get_value(arg, &v);
    char tag[32];
    strncpy(tag, v.value.str, sizeof tag - 1);
    tag[sizeof tag - 1] = 0;
    vpi_free_object(arg);
    vpi_free_object(it);
    if (count) {
        (*count)++;
        vpi_printf("CHK|tick|%s|%d\n", tag, *count);
    }
    CHECK(vpi_get(vpiUserDefn, call) == 1, "vpiUserDefn of a call");
    vpi_free_object(call);
    return 0;
}

static PLI_INT32 size_sized(PLI_BYTE8 *ud) { return 12; }
static PLI_INT32 call_sized(PLI_BYTE8 *ud) { return 0; }

static PLI_INT32 probe_call(PLI_BYTE8 *ud) {
    s_vpi_systf_data d;
    s_vpi_error_info e;

    /* The handle vpi_register_systf returned. */
    CHECK(tick_reg != NULL, "vpi_register_systf returns a handle");
    CHECK(vpi_get(vpiType, tick_reg) == vpiUserSystf, "its type is vpiUserSystf");
    memset(&d, 0, sizeof d);
    vpi_get_systf_info(tick_reg, &d);
    CHECK(d.type == vpiSysTask, "info: type");
    CHECK(d.tfname && strcmp(d.tfname, "$tick") == 0, "info: tfname");
    CHECK(d.calltf == tick_call && d.compiletf == tick_compile, "info: routines");
    CHECK(d.sizetf == NULL, "info: sizetf");
    CHECK(d.user_data == (PLI_BYTE8 *)0x1234, "info: user_data");

    /* Through the call handle. */
    vpiHandle call = vpi_handle(vpiSysTfCall, NULL);
    vpiHandle me = vpi_handle(vpiUserSystf, call);
    CHECK(me != NULL, "vpi_handle(vpiUserSystf, call)");
    memset(&d, 0, sizeof d);
    vpi_get_systf_info(me, &d);
    CHECK(d.tfname && strcmp(d.tfname, "$probe") == 0, "info through the call handle");
    CHECK(d.calltf == probe_call, "info through the call handle: calltf");

    /* Every registration, in order. */
    vpiHandle it = vpi_iterate(vpiUserSystf, NULL);
    vpiHandle s;
    vpi_printf("CHK|systfs");
    while ((s = vpi_scan(it)) != NULL) {
        memset(&d, 0, sizeof d);
        vpi_get_systf_info(s, &d);
        vpi_printf("|%s:%d:%d", d.tfname, d.type, d.sysfunctype);
        if (strcmp(d.tfname, "$sized") == 0)
            CHECK(d.sizetf == size_sized, "info: a sized function's sizetf");
        vpi_free_object(s);
    }
    vpi_printf("\n");

    /* Misuse is reported, not ignored. */
    CHECK(vpi_put_userdata(NULL, (void *)1) == 0, "put_userdata(NULL) fails");
    CHECK(vpi_chk_error(&e) == vpiError, "... and reports an error");
    CHECK(vpi_put_userdata(tick_reg, (void *)1) == 0, "put_userdata on a vpiUserSystf fails");
    CHECK(vpi_chk_error(NULL) == vpiError, "... and reports an error");
    CHECK(vpi_get_userdata(tick_reg) == NULL, "get_userdata on a vpiUserSystf is NULL");
    CHECK(vpi_chk_error(NULL) == vpiError, "... and reports an error");
    memset(&d, 0, sizeof d);
    vpi_get_systf_info(call, &d);
    CHECK(d.tfname == NULL && vpi_chk_error(NULL) == vpiError,
          "get_systf_info on a call handle is an error");
    CHECK(vpi_get_userdata(call) == NULL, "a call instance without put data reads NULL");

    /* No save or restart is ever in progress. */
    char buf[8] = "abcdefg";
    CHECK(vpi_put_data(1, buf, 4) == 0, "vpi_put_data outside a save returns 0");
    CHECK(vpi_chk_error(&e) == vpiError, "... and reports an error");
    CHECK(vpi_get_data(1, buf, 4) == 0, "vpi_get_data outside a restart returns 0");
    CHECK(vpi_chk_error(NULL) == vpiError, "... and reports an error");
    CHECK(strcmp(buf, "abcdefg") == 0, "vpi_get_data wrote nothing");

    vpi_printf("CHK|compiles|%d\n", compiles);
    vpi_free_object(call);
    return 0;
}

static vpiHandle reg(PLI_INT32 type, PLI_INT32 ftype, const char *name,
                     PLI_INT32 (*calltf)(PLI_BYTE8 *), PLI_INT32 (*compiletf)(PLI_BYTE8 *),
                     PLI_INT32 (*sizetf)(PLI_BYTE8 *), PLI_BYTE8 *ud) {
    s_vpi_systf_data d;
    memset(&d, 0, sizeof d);
    d.type = type;
    d.sysfunctype = ftype;
    d.tfname = (PLI_BYTE8 *)name;
    d.calltf = calltf;
    d.compiletf = compiletf;
    d.sizetf = sizetf;
    d.user_data = ud;
    return vpi_register_systf(&d);
}

static void startup(void) {
    tick_reg = reg(vpiSysTask, 0, "$tick", tick_call, tick_compile, NULL, (PLI_BYTE8 *)0x1234);
    reg(vpiSysTask, 0, "$probe", probe_call, NULL, NULL, NULL);
    reg(vpiSysFunc, vpiSizedFunc, "$sized", call_sized, NULL, size_sized, NULL);
}

void (*vlog_startup_routines[])(void) = { startup, 0 };
"#;

const SYSTF_SV: &str = r#"
module leaf;
  initial begin
    repeat (2) $tick("leaf");
  end
endmodule
module top;
  leaf u0();
  leaf u1();
  initial begin
    repeat (3) $tick("loop");
    $tick("once");
    #5;
    $tick("loop2");
    $probe;
    $finish;
  end
endmodule
"#;

#[test]
fn vpi_userdata_systf_info_and_save_restart_data() {
    let d = scratch("vpi_systf_info");
    let lib = shared_lib(&d, "systf", SYSTF_C);
    let text = run(&d, "--vpi-lib", &lib, SYSTF_SV, &[]);
    let _ = std::fs::remove_dir_all(&d);
    expect(
        &text,
        &[
            // One call site in a loop is one call instance...
            "CHK|tick|loop|1",
            "CHK|tick|loop|2",
            "CHK|tick|loop|3",
            // ...every other site (and each module instance) its own.
            "CHK|tick|once|1",
            "CHK|tick|loop2|1",
            "CHK|tick|leaf|2",
            "CHK|systfs|$tick:1:0|$probe:1:0|$sized:2:4",
            // compiletf once per call instance: loop, once, loop2, u0, u1.
            "CHK|compiles|5",
        ],
    );
    assert!(!text.contains("CHK|tick|loop|4"), "{text}");
    assert!(!text.contains("CHK|tick|leaf|3"), "{text}");
    assert_eq!(text.matches("CHK|tick|leaf|2").count(), 2, "{text}");
}

// ---------------------------------------------------------------------------
// vpi_handle_by_multi_index, vpi_get_value_array, vpi_put_value_array.
// ---------------------------------------------------------------------------

const ARRAYS_C: &str = r#"
#include <string.h>
#include "vpi_user.h"

#define CHECK(c, m) do { if (!(c)) vpi_printf("FAIL: %s\n", (m)); } while (0)

static vpiHandle H(const char *n) { return vpi_handle_by_name((PLI_BYTE8 *)n, NULL); }

static int ival(vpiHandle h) {
    s_vpi_value v;
    v.format = vpiIntVal;
    vpi_get_value(h, &v);
    return v.format == vpiIntVal ? v.value.integer : -99999;
}

/* A section read that must fail: NULL value pointer plus an error. */
static void bad_get(vpiHandle a, PLI_INT32 fmt, PLI_INT32 *idx, PLI_UINT32 n, const char *what) {
    s_vpi_arrayvalue av;
    int dummy[8];
    av.format = fmt;
    av.flags = vpiUserAllocFlag;
    av.value.integers = dummy;
    vpi_get_value_array(a, &av, idx, n);
    CHECK(av.value.integers == NULL, what);
    CHECK(vpi_chk_error(NULL) == vpiError, what);
}

void arrays_read(void) {
    s_vpi_arrayvalue av;
    PLI_INT32 idx[3];

    /* One dimension, declared [3:0]: sections run 3 -> 0. */
    vpiHandle m1 = H("top.m1");
    CHECK(m1 && vpi_get(vpiType, m1) == vpiRegArray, "m1 is a vpiRegArray");
    av.format = vpiIntVal; av.flags = 0; av.value.integers = NULL;
    idx[0] = 3;
    vpi_get_value_array(m1, &av, idx, 4);
    if (av.value.integers)
        vpi_printf("CHK|m1_int|%d,%d,%d,%d\n", av.value.integers[0], av.value.integers[1],
                   av.value.integers[2], av.value.integers[3]);
    s_vpi_vecval vv[2];
    av.format = vpiVectorVal; av.flags = vpiUserAllocFlag; av.value.vectors = vv;
    idx[0] = 1;
    vpi_get_value_array(m1, &av, idx, 2);
    CHECK(av.value.vectors == vv, "vpiUserAllocFlag keeps the caller's buffer");
    vpi_printf("CHK|m1_vec|%x,%x\n", (unsigned)vv[0].aval, (unsigned)vv[1].aval);
    s_vpi_time tv[2];
    av.format = vpiTimeVal; av.flags = vpiUserAllocFlag; av.value.times = tv;
    vpi_get_value_array(m1, &av, idx, 2);
    vpi_printf("CHK|m1_time|%u,%u\n", tv[0].low, tv[1].low);
    bad_get(m1, vpiIntVal, idx, 3, "a section past the end is an error");
    bad_get(m1, vpiStringVal, idx, 1, "vpiStringVal is not an array format");
    bad_get(m1, vpiLongIntVal, idx, 1, "vpiLongIntVal does not suit logic elements");
    bad_get(m1, vpiIntVal, idx, 0, "num 0 is an error");
    idx[0] = 4;
    bad_get(m1, vpiIntVal, idx, 1, "a start index out of range is an error");
    bad_get(H("top.hv"), vpiIntVal, idx, 1, "a vector is not an array");

    /* Two dimensions, int m2 [0:2][3:0]. */
    vpiHandle m2 = H("top.m2");
    CHECK(m2 && vpi_get(vpiType, m2) == vpiRegArray, "m2 is a vpiRegArray");
    CHECK(vpi_get(vpiSize, m2) == 12, "vpiSize of m2 counts every element");
    av.format = vpiIntVal; av.flags = 0; av.value.integers = NULL;
    idx[0] = 1; idx[1] = 1;
    vpi_get_value_array(m2, &av, idx, 4);
    if (av.value.integers)
        vpi_printf("CHK|m2_int|%d,%d,%d,%d\n", av.value.integers[0], av.value.integers[1],
                   av.value.integers[2], av.value.integers[3]);
    vpiHandle row = H("top.m2[1]");
    CHECK(row && vpi_get(vpiType, row) == vpiRegArray && vpi_get(vpiSize, row) == 4,
          "a sub-array by name");
    idx[0] = 3;
    av.value.integers = NULL;
    vpi_get_value_array(row, &av, idx, 4);
    if (av.value.integers)
        vpi_printf("CHK|row_int|%d,%d,%d,%d\n", av.value.integers[0], av.value.integers[1],
                   av.value.integers[2], av.value.integers[3]);
    vpiHandle row2 = vpi_handle_by_index(m2, 2);
    CHECK(row2 && strcmp(vpi_get_str(vpiFullName, row2), "top.m2[2]") == 0,
          "vpi_handle_by_index on a 2-D array gives a sub-array");
    vpiHandle e20 = vpi_handle_by_index(row2, 0);
    vpi_printf("CHK|m2[2][0]|%d\n", e20 ? ival(e20) : -1);
    idx[0] = 2; idx[1] = 3;
    vpiHandle e23 = vpi_handle_by_multi_index(m2, 2, idx);
    CHECK(e23 && strcmp(vpi_get_str(vpiFullName, e23), "top.m2[2][3]") == 0, "multi-index element");
    CHECK(e23 && vpi_get(vpiType, e23) == vpiIntVar, "an int element is a vpiIntVar");
    vpi_printf("CHK|m2[2][3]|%d\n", e23 ? ival(e23) : -1);
    idx[2] = 4;
    vpiHandle b = vpi_handle_by_multi_index(m2, 3, idx);
    CHECK(b && vpi_get(vpiType, b) == vpiRegBit && vpi_get(vpiSize, b) == 1, "element bit-select");
    vpi_printf("CHK|m2[2][3][4]|%d\n", b ? ival(b) : -1);
    idx[0] = 3; idx[1] = 0;
    CHECK(vpi_handle_by_multi_index(m2, 2, idx) == NULL, "an index out of range gives NULL");
    CHECK(vpi_handle_by_multi_index(m2, 4, idx) == NULL, "too many indices give NULL");

    /* Three... no: logic [41:0] m3 [2:0][3:5], the LRM's own example. */
    vpiHandle m3 = H("top.m3");
    PLI_BYTE8 raw[6 * 2 * 5];
    av.format = vpiRawFourStateVal; av.flags = vpiUserAllocFlag; av.value.rawvals = raw;
    idx[0] = 1; idx[1] = 4;
    vpi_get_value_array(m3, &av, idx, 5);
    vpi_printf("CHK|m3_raw4");
    for (int k = 0; k < 5; k++)
        vpi_printf("|%02x/%02x", (unsigned char)raw[k * 12], (unsigned char)raw[k * 12 + 6]);
    vpi_printf("\n");
    av.format = vpiRawTwoStateVal;
    vpi_get_value_array(m3, &av, idx, 5);
    vpi_printf("CHK|m3_raw2");
    for (int k = 0; k < 5; k++)
        vpi_printf("|%02x", (unsigned char)raw[k * 6]);
    vpi_printf("\n");

    /* The SystemVerilog integer formats. */
    PLI_INT16 sh[4];
    av.format = vpiShortIntVal; av.flags = vpiUserAllocFlag; av.value.shortints = sh;
    idx[0] = 0;
    vpi_get_value_array(H("top.s1"), &av, idx, 4);
    vpi_printf("CHK|s1_short|%d,%d,%d,%d\n", sh[0], sh[1], sh[2], sh[3]);
    vpi_get_value_array(H("top.b1"), &av, idx, 4);
    vpi_printf("CHK|b1_short|%d,%d,%d,%d\n", sh[0], sh[1], sh[2], sh[3]);
    PLI_INT64 lg[4];
    av.format = vpiLongIntVal; av.value.longints = lg;
    vpi_get_value_array(H("top.l1"), &av, idx, 2);
    vpi_printf("CHK|l1_long|%llx,%lld\n", (unsigned long long)lg[0], (long long)lg[1]);
    vpi_get_value_array(H("top.s1"), &av, idx, 2);
    vpi_printf("CHK|s1_long|%lld,%lld\n", (long long)lg[0], (long long)lg[1]);
    bad_get(H("top.l1"), vpiShortIntVal, idx, 1, "vpiShortIntVal does not suit longint");
    av.format = vpiIntVal; av.flags = 0; av.value.integers = NULL;
    vpi_get_value_array(H("top.b1"), &av, idx, 2);
    if (av.value.integers)
        vpi_printf("CHK|b1_int|%d,%d\n", av.value.integers[0], av.value.integers[1]);

    /* Reals. */
    double rd[3];
    av.format = vpiRealVal; av.flags = vpiUserAllocFlag; av.value.reals = rd;
    vpi_get_value_array(H("top.r1"), &av, idx, 3);
    vpi_printf("CHK|r1_real|%g,%g,%g\n", rd[0], rd[1], rd[2]);
    float fl[2];
    av.format = vpiShortRealVal; av.value.shortreals = fl;
    vpi_get_value_array(H("top.sr"), &av, idx, 2);
    vpi_printf("CHK|sr_short|%g,%g\n", fl[0], fl[1]);
    bad_get(H("top.r1"), vpiShortRealVal, idx, 1, "vpiShortRealVal does not suit real");
    bad_get(H("top.r1"), vpiIntVal, idx, 1, "vpiIntVal does not suit real");

    /* An array too large to have named elements. */
    vpiHandle big = H("top.big");
    CHECK(big && vpi_get(vpiSize, big) == 200000, "the large array by name");
    vpiHandle b1 = H("top.big[150001]");
    vpi_printf("CHK|big[150001]|%d\n", b1 ? ival(b1) : -1);
    av.format = vpiIntVal; av.flags = 0; av.value.integers = NULL;
    idx[0] = 150000;
    vpi_get_value_array(big, &av, idx, 3);
    if (av.value.integers)
        vpi_printf("CHK|big_int|%d,%d,%d\n", av.value.integers[0], av.value.integers[1],
                   av.value.integers[2]);

    /* A bit of a plain vector declared [15:8]. */
    vpiHandle hv = H("top.hv");
    idx[0] = 9;
    vpiHandle hb = vpi_handle_by_multi_index(hv, 1, idx);
    vpi_printf("CHK|hv[9]|%d|%s\n", hb ? ival(hb) : -1, hb ? vpi_get_str(vpiFullName, hb) : "-");
    idx[0] = 7;
    CHECK(vpi_handle_by_multi_index(hv, 1, idx) == NULL, "a bit outside [15:8] gives NULL");

    /* Packed dimensions: logic [3:0][7:0] pk, and an array of them. */
    vpiHandle pk = H("top.pk");
    idx[0] = 1;
    vpiHandle p1 = vpi_handle_by_multi_index(pk, 1, idx);
    CHECK(p1 && vpi_get(vpiType, p1) == vpiPartSelect && vpi_get(vpiSize, p1) == 8,
          "one packed index selects a byte");
    idx[0] = 2; idx[1] = 4;
    vpiHandle p24 = vpi_handle_by_multi_index(pk, 2, idx);
    CHECK(p24 && vpi_get(vpiType, p24) == vpiRegBit, "two packed indices select a bit");
    idx[1] = 3;
    vpi_printf("CHK|pk|%x|%d|%d\n", p1 ? ival(p1) : -1, p24 ? ival(p24) : -1,
               ival(vpi_handle_by_multi_index(pk, 2, idx)));
    idx[0] = 1; idx[1] = 0; idx[2] = 2;
    vpiHandle apb = vpi_handle_by_multi_index(H("top.ap"), 3, idx);
    vpi_printf("CHK|ap[1][0][2]|%d|%s\n", apb ? ival(apb) : -1,
               apb ? vpi_get_str(vpiFullName, apb) : "-");
}

void arrays_write(void) {
    s_vpi_arrayvalue av;
    PLI_INT32 idx[2];

    /* §38.35's example: five 42-bit values into m3 from [1][4], the last
     * one with x in its low byte, propagation off. */
    PLI_BYTE8 buf[6 * 2 * 5];
    memset(buf, 0, sizeof buf);
    int off = 0;
    for (int e = 1; e <= 5; e++) {
        for (int i = 0; i < e; i++)
            buf[off + i] = 1;
        off += 12;
    }
    off -= 12;
    buf[off] = (PLI_BYTE8)0xff;
    buf[off + 6] = (PLI_BYTE8)0xff;
    av.format = vpiRawFourStateVal; av.flags = vpiPropagateOff; av.value.rawvals = buf;
    idx[0] = 1; idx[1] = 4;
    vpi_put_value_array(vpi_handle_by_name("top.m3", NULL), &av, idx, 5);

    /* One value to a section. */
    PLI_INT32 seven = 0x77;
    av.format = vpiIntVal; av.flags = vpiOneValue; av.value.integers = &seven;
    idx[0] = 1;
    vpi_put_value_array(vpi_handle_by_name("top.m1", NULL), &av, idx, 2);

    PLI_INT16 sh[2] = { 100, -100 };
    av.format = vpiShortIntVal; av.flags = 0; av.value.shortints = sh;
    idx[0] = 1;
    vpi_put_value_array(vpi_handle_by_name("top.s1", NULL), &av, idx, 2);
    idx[0] = 0;
    vpi_put_value_array(vpi_handle_by_name("top.l1", NULL), &av, idx, 2);
    PLI_INT64 lg = -5;
    av.format = vpiLongIntVal; av.value.longints = &lg;
    idx[0] = 1;
    vpi_put_value_array(vpi_handle_by_name("top.l1", NULL), &av, idx, 1);
    double rd[2] = { 3.25, -0.5 };
    av.format = vpiRealVal; av.value.reals = rd;
    idx[0] = 1;
    vpi_put_value_array(vpi_handle_by_name("top.r1", NULL), &av, idx, 2);
    float fl = 2.75f;
    av.format = vpiShortRealVal; av.flags = vpiOneValue; av.value.shortreals = &fl;
    idx[0] = 0;
    vpi_put_value_array(vpi_handle_by_name("top.sr", NULL), &av, idx, 2);
    PLI_INT32 m7 = -7;
    av.format = vpiIntVal; av.flags = vpiOneValue; av.value.integers = &m7;
    vpi_put_value_array(vpi_handle_by_name("top.b1", NULL), &av, idx, 4);
    PLI_BYTE8 two[2] = { 3, 4 };
    av.format = vpiRawTwoStateVal; av.flags = 0; av.value.rawvals = two;
    idx[0] = 199998;
    vpi_put_value_array(vpi_handle_by_name("top.big", NULL), &av, idx, 2);

    /* Refused puts write nothing. */
    PLI_INT16 one = 1;
    av.format = vpiShortIntVal; av.flags = 0; av.value.shortints = &one;
    idx[0] = 0;
    vpi_put_value_array(vpi_handle_by_name("top.b1", NULL), &av, idx, 1);
    CHECK(vpi_chk_error(NULL) == vpiError, "vpiShortIntVal cannot be put to byte elements");
    av.format = vpiIntVal; av.flags = vpiUserAllocFlag; av.value.integers = &seven;
    vpi_put_value_array(vpi_handle_by_name("top.b1", NULL), &av, idx, 1);
    CHECK(vpi_chk_error(NULL) == vpiError, "vpiUserAllocFlag is not a put flag");
    av.flags = 0;
    idx[0] = 2;
    vpi_put_value_array(vpi_handle_by_name("top.b1", NULL), &av, idx, 3);
    CHECK(vpi_chk_error(NULL) == vpiError, "a put past the end is an error");

    /* Propagation off: the value lands, the watcher does not run. */
    PLI_INT32 v55 = 0x55;
    av.format = vpiIntVal; av.flags = vpiPropagateOff; av.value.integers = &v55;
    idx[0] = 2;
    vpi_put_value_array(vpi_handle_by_name("top.m1", NULL), &av, idx, 1);
}

void arrays_touch(void) {
    s_vpi_arrayvalue av;
    PLI_INT32 idx[1] = { 2 };
    PLI_INT32 v66 = 0x66;
    av.format = vpiIntVal; av.flags = 0; av.value.integers = &v66;
    vpi_put_value_array(vpi_handle_by_name("top.m1", NULL), &av, idx, 1);
}
"#;

const ARRAYS_SV: &str = r#"
module top;
  logic [7:0] m1 [3:0];
  int m2 [0:2][3:0];
  logic [41:0] m3 [2:0][3:5];
  shortint s1 [0:3];
  longint l1 [0:1];
  real r1 [0:2];
  shortreal sr [0:1];
  byte b1 [0:3];
  logic [3:0] big [0:199999];
  logic [15:8] hv;
  logic [3:0][7:0] pk;
  logic [1:0][3:0] ap [0:1];
  int hits = 0;
  // A continuous assignment reads the element; its change wakes the always.
  wire [7:0] w2 = m1[2];
  always @(w2) hits = hits + 1;
  import "DPI-C" context function void arrays_read();
  import "DPI-C" context function void arrays_write();
  import "DPI-C" context function void arrays_touch();
  initial begin
    for (int i = 0; i < 4; i++) m1[i] = 8'h10 + i;
    for (int i = 0; i < 3; i++) for (int j = 0; j < 4; j++) m2[i][j] = i * 10 + j;
    for (int i = 0; i < 3; i++) for (int j = 3; j <= 5; j++) m3[i][j] = 42'(i * 100 + j);
    m3[0][5] = {38'd5, 4'bxz10};
    s1[0] = -2; s1[1] = -1; s1[2] = 1; s1[3] = 2;
    l1[0] = 64'h1234_5678_9abc_def0; l1[1] = -3;
    r1[0] = 1.5; r1[1] = -2.25; r1[2] = 1e10;
    sr[0] = 0.5; sr[1] = -1.25;
    b1[0] = -128; b1[1] = -1; b1[2] = 0; b1[3] = 127;
    big[150000] = 4'ha; big[150001] = 4'hb; big[150002] = 4'hc;
    hv = 8'b0000_0010;
    pk = 32'h44332211;
    ap[1] = 8'ha5;
    #1 arrays_read();
    $display("H0 hits=%0d", hits);
    arrays_write();
    #1;
    $display("H1 hits=%0d m1[2]=%h w2=%h", hits, m1[2], w2);
    $display("W m3 %h %h %h %h %h", m3[1][4], m3[1][5], m3[0][3], m3[0][4], m3[0][5]);
    $display("W m1 %h %h %h %h", m1[3], m1[2], m1[1], m1[0]);
    $display("W s1 %0d %0d %0d %0d", s1[0], s1[1], s1[2], s1[3]);
    $display("W l1 %0d %0d", l1[0], l1[1]);
    $display("W r1 %g %g %g", r1[0], r1[1], r1[2]);
    $display("W sr %g %g", sr[0], sr[1]);
    $display("W b1 %0d %0d %0d %0d", b1[0], b1[1], b1[2], b1[3]);
    $display("W big %h %h", big[199998], big[199999]);
    arrays_touch();
    #1 $display("H2 hits=%0d m1[2]=%h w2=%h", hits, m1[2], w2);
    $finish;
  end
endmodule
"#;

fn arrays_expected() -> Vec<&'static str> {
    vec![
        "CHK|m1_int|19,18,17,16",
        "CHK|m1_vec|11,10",
        "CHK|m1_time|17,16",
        "CHK|m2_int|11,10,23,22",
        "CHK|row_int|13,12,11,10",
        "CHK|m2[2][0]|20",
        "CHK|m2[2][3]|23",
        "CHK|m2[2][3][4]|1",
        // a[1][4], a[1][5], a[0][3], a[0][4], a[0][5] (§38.16's order);
        // the last one's low nibble is x z 1 0.
        "CHK|m3_raw4|68/00|69/00|03/00|04/00|5a/0c",
        "CHK|m3_raw2|68|69|03|04|52",
        "CHK|s1_short|-2,-1,1,2",
        "CHK|b1_short|-128,-1,0,127",
        "CHK|l1_long|123456789abcdef0,-3",
        "CHK|s1_long|-2,-1",
        "CHK|b1_int|-128,-1",
        "CHK|r1_real|1.5,-2.25,1e+10",
        "CHK|sr_short|0.5,-1.25",
        "CHK|big[150001]|11",
        "CHK|big_int|10,11,12",
        "CHK|hv[9]|1|top.hv[9]",
        "CHK|pk|22|1|0",
        "CHK|ap[1][0][2]|1|top.ap[1][0][2]",
        "W m3 00000000001 00000000101 00000010101 00001010101 001010101xx",
        "W m1 13 55 77 77",
        "W s1 -2 100 -100 2",
        "W l1 100 -5",
        "W r1 1.5 3.25 -0.5",
        "W sr 2.75 2.75",
        "W b1 -7 -7 -7 -7",
        "W big 3 4",
    ]
}

#[test]
fn vpi_multi_index_and_value_arrays() {
    let d = scratch("vpi_arrays");
    let lib = shared_lib(&d, "arrays", ARRAYS_C);
    let text = run(&d, "--dpi-lib", &lib, ARRAYS_SV, &[]);
    expect(&text, &arrays_expected());
    // vpiPropagateOff: the value landed without reaching the continuous
    // assignment that reads it; a normal put then does, once.
    let hits = |tag: &str| -> i64 {
        let line = text.lines().find(|l| l.starts_with(tag)).unwrap_or("");
        line.split("hits=")
            .nth(1)
            .and_then(|r| r.split_whitespace().next())
            .and_then(|n| n.parse().ok())
            .unwrap_or(-1)
    };
    assert!(text.contains("m1[2]=55 w2=12"), "{text}");
    assert_eq!(
        hits("H1"),
        hits("H0"),
        "vpiPropagateOff woke the watcher:\n{text}"
    );
    assert_eq!(
        hits("H2"),
        hits("H0") + 1,
        "a normal put must wake it:\n{text}"
    );
    assert!(text.contains("m1[2]=66 w2=66"), "{text}");
    // The same checks with the large array in the packed memory arena.
    let packed = run(
        &d,
        "--dpi-lib",
        &lib,
        ARRAYS_SV,
        &[("XEZIM_PACKED_MEM", "1")],
    );
    let _ = std::fs::remove_dir_all(&d);
    expect(&packed, &arrays_expected());
}

// ---------------------------------------------------------------------------
// vpiStrengthVal and the other vpi_get_value / vpi_put_value formats.
// ---------------------------------------------------------------------------

const FORMATS_C: &str = r#"
#include <string.h>
#include "vpi_user.h"

#define CHECK(c, m) do { if (!(c)) vpi_printf("FAIL: %s\n", (m)); } while (0)

static vpiHandle H(const char *n) { return vpi_handle_by_name((PLI_BYTE8 *)n, NULL); }

static void strength(const char *n) {
    s_vpi_value v;
    v.format = vpiStrengthVal;
    vpiHandle h = H(n);
    vpi_get_value(h, &v);
    if (v.format != vpiStrengthVal) {
        vpi_printf("FAIL: vpiStrengthVal on %s\n", n);
        return;
    }
    vpi_printf("CHK|str|%s", n);
    for (int i = 0; i < vpi_get(vpiSize, h); i++)
        vpi_printf("|%d:%x:%x", v.value.strength[i].logic, v.value.strength[i].s0,
                   v.value.strength[i].s1);
    vpi_printf("\n");
}

static void str(const char *n, PLI_INT32 fmt) {
    s_vpi_value v;
    v.format = fmt;
    vpi_get_value(H(n), &v);
    vpi_printf("CHK|s%d|%s|%s\n", fmt, n, v.format == fmt ? v.value.str : "?");
}

static int ival(const char *n, PLI_INT32 fmt) {
    s_vpi_value v;
    v.format = fmt;
    vpi_get_value(H(n), &v);
    if (v.format != fmt)
        vpi_printf("FAIL: format %d on %s\n", fmt, n);
    return v.value.integer;
}

static PLI_INT32 objtype(const char *n) {
    s_vpi_value v;
    v.format = vpiObjTypeVal;
    vpi_get_value(H(n), &v);
    return v.format;
}

static void put(const char *n, s_vpi_value *v) {
    vpi_put_value(H(n), v, NULL, vpiNoDelay);
}

void formats_probe(void) {
    s_vpi_value v;

    /* Strengths: a pull-driven net, a supply-driven one, a variable, a
     * vector net with a z bit, an undriven net. */
    strength("top.pw");
    strength("top.pz");
    strength("top.r1");
    strength("top.nv");
    strength("top.zn");

    vpi_printf("CHK|short|%d|%d\n", ival("top.si", vpiShortIntVal), ival("top.iv", vpiShortIntVal));
    v.format = vpiLongIntVal;
    vpi_get_value(H("top.li"), &v);
    vpi_printf("CHK|long|%lld", v.format == vpiLongIntVal ? *(PLI_INT64 *)v.value.misc : 0LL);
    v.format = vpiLongIntVal;
    vpi_get_value(H("top.wide"), &v);
    vpi_printf("|%llx\n", v.format == vpiLongIntVal ? (unsigned long long)*(PLI_INT64 *)v.value.misc : 0ULL);
    v.format = vpiShortRealVal;
    vpi_get_value(H("top.srv"), &v);
    vpi_printf("CHK|shortreal|%g", v.value.real);
    v.format = vpiShortRealVal;
    vpi_get_value(H("top.rs"), &v);
    vpi_printf("|%.9g\n", v.value.real);
    v.format = vpiRawFourStateVal;
    vpi_get_value(H("top.hx1"), &v);
    vpi_printf("CHK|raw4|%02x/%02x", (unsigned char)v.value.misc[0], (unsigned char)v.value.misc[1]);
    v.format = vpiRawTwoStateVal;
    vpi_get_value(H("top.hx1"), &v);
    vpi_printf("|raw2|%02x\n", (unsigned char)v.value.misc[0]);

    /* vpiIntVal sign-extends a signed value and rounds a real. */
    vpi_printf("CHK|int|%d|%d|%d|%d\n", ival("top.bn", vpiIntVal), ival("top.rv", vpiIntVal),
               ival("top.rn", vpiIntVal), ival("top.iv", vpiIntVal));
    str("top.rs", vpiStringVal);
    str("top.rv", vpiStringVal);
    str("top.hx1", vpiHexStrVal);
    str("top.hx2", vpiHexStrVal);
    str("top.hx3", vpiHexStrVal);
    str("top.ox", vpiOctStrVal);
    str("top.rv", vpiDecStrVal);

    vpi_printf("CHK|objtype|%d|%d|%d|%d|%d|%d|%d|%d\n", objtype("top.iv"), objtype("top.hx1"),
               objtype("top.one"), objtype("top.rv"), objtype("top.tv"), objtype("top.ig"),
               objtype("top.si"), objtype("top.li"));

    /* Puts. */
    s_vpi_strengthval sv;
    sv.logic = vpi1; sv.s0 = vpiStrongDrive; sv.s1 = vpiStrongDrive;
    v.format = vpiStrengthVal; v.value.strength = &sv;
    put("top.sc", &v);
    CHECK(vpi_chk_error(NULL) == 0, "vpiStrengthVal put to a scalar");
    put("top.vec4", &v);
    CHECK(vpi_chk_error(NULL) == vpiError, "vpiStrengthVal put to a vector is an error");
    sv.s0 = 0;
    put("top.sl", &v);
    CHECK(vpi_chk_error(NULL) == vpiError, "a strength code of 0 is an error");
    v.format = vpiScalarVal; v.value.scalar = vpiL;
    put("top.sl", &v);
    v.value.scalar = vpiH;
    put("top.sh", &v);
    v.format = vpiShortIntVal; v.value.integer = -2;
    put("top.si", &v);
    PLI_INT64 big = 0x100000001LL;
    v.format = vpiLongIntVal; v.value.misc = (PLI_BYTE8 *)&big;
    put("top.li", &v);
    v.format = vpiShortRealVal; v.value.real = 0.25;
    put("top.srv", &v);
    PLI_BYTE8 raw[2] = { 0x5, 0x3 };
    v.format = vpiRawFourStateVal; v.value.misc = raw;
    put("top.vec4", &v);
    PLI_BYTE8 raw2 = (PLI_BYTE8)0xa5;
    v.format = vpiRawTwoStateVal; v.value.misc = &raw2;
    put("top.hx3", &v);
    s_vpi_time t;
    t.type = vpiSimTime; t.high = 1; t.low = 2;
    v.format = vpiTimeVal; v.value.time = &t;
    put("top.tv", &v);
    v.format = vpiRealVal; v.value.real = 3.7;
    put("top.ri", &v);
    v.format = vpiIntVal; v.value.integer = 5;
    put("top.rr", &v);
    v.format = vpiObjTypeVal;
    put("top.ri", &v);
    CHECK(vpi_chk_error(NULL) == vpiError, "vpiObjTypeVal cannot be put");
}
"#;

const FORMATS_SV: &str = r#"
module top;
  reg en = 1;
  wire pw;
  assign (pull1, strong0) pw = en;
  wire pz;
  assign (weak1, supply0) pz = 1'b0;
  reg r1 = 1;
  wire [3:0] nv;
  assign nv = 4'b10z0;
  wire zn;
  shortint si = -300;
  longint li = -5;
  shortreal srv = 1.5;
  real rv = 2.5, rn = -2.5, rs = 0.1;
  byte bn = -1;
  time tv = 10;
  int iv = -7;
  logic [7:0] hx1 = 8'b1x10_zzzz;
  logic [7:0] hx2 = 8'bxxxx_xxxx;
  logic [7:0] hx3 = 8'b0z11_0000;
  logic [11:0] ox = 12'b000_xz1_zzz_111;
  logic one = 1'b1;
  logic [41:0] wide = 42'h3_0000_00ff;
  integer ig = 5;
  reg sc = 0, sl = 1, sh = 0;
  reg [3:0] vec4 = 0;
  int ri;
  real rr;
  import "DPI-C" context function void formats_probe();
  initial begin
    #1 formats_probe();
    #1;
    $display("P sc=%b sl=%b sh=%b vec4=%b si=%0d li=%0d srv=%g", sc, sl, sh, vec4, si, li, srv);
    $display("P hx3=%h tv=%0d ri=%0d rr=%g", hx3, tv, ri, rr);
    $finish;
  end
endmodule
"#;

#[test]
fn vpi_strength_and_value_formats() {
    let d = scratch("vpi_formats");
    let lib = shared_lib(&d, "formats", FORMATS_C);
    let text = run(&d, "--dpi-lib", &lib, FORMATS_SV, &[]);
    let _ = std::fs::remove_dir_all(&d);
    expect(
        &text,
        &[
            // logic:s0:s1 per bit, LSB first (vpi1 = 1, vpiZ = 2; pull 0x20,
            // strong 0x40, supply 0x80, HiZ 0x01).
            "CHK|str|top.pw|1:20:20",
            "CHK|str|top.pz|0:80:80",
            "CHK|str|top.r1|1:40:40",
            "CHK|str|top.nv|0:40:40|2:1:1|0:40:40|1:40:40",
            "CHK|str|top.zn|2:1:1",
            "CHK|short|-300|-7",
            "CHK|long|-5|3000000ff",
            "CHK|shortreal|1.5|0.100000001",
            "CHK|raw4|e0/4f|raw2|a0",
            "CHK|int|-1|3|-3|-7",
            "CHK|s8|top.rs|0.1",
            "CHK|s8|top.rv|2.5",
            "CHK|s4|top.hx1|Xz",
            "CHK|s4|top.hx2|xx",
            "CHK|s4|top.hx3|Z0",
            "CHK|s2|top.ox|0Xz7",
            "CHK|s3|top.rv|3",
            // vpiIntVal, vpiVectorVal, vpiScalarVal, vpiRealVal, vpiTimeVal,
            // vpiIntVal (integer), vpiIntVal (shortint), vpiVectorVal (longint).
            "CHK|objtype|6|9|5|7|11|6|6|9",
            "P sc=1 sl=0 sh=1 vec4=01zx si=-2 li=4294967297 srv=0.25",
            "P hx3=a5 tv=4294967298 ri=4 rr=5",
        ],
    );
}

// ---------------------------------------------------------------------------
// vpi_get_delays / vpi_put_delays, vpi_handle_multi(vpiInterModPath).
// ---------------------------------------------------------------------------

const DELAYS_C: &str = r#"
#include <string.h>
#include "vpi_user.h"

#define CHECK(c, m) do { if (!(c)) vpi_printf("FAIL: %s\n", (m)); } while (0)

static vpiHandle H(const char *n) { return vpi_handle_by_name((PLI_BYTE8 *)n, NULL); }

static s_vpi_time da[18];
static s_vpi_delay dl;

static void setup(int n, PLI_INT32 time_type) {
    memset(da, 0, sizeof da);
    memset(&dl, 0, sizeof dl);
    dl.da = da;
    dl.no_of_delays = n;
    dl.time_type = time_type;
}

static void show(const char *tag, vpiHandle h, int n) {
    setup(n, vpiSimTime);
    vpi_get_delays(h, &dl);
    vpi_printf("CHK|%s", tag);
    for (int i = 0; i < n; i++)
        vpi_printf("|%d", (int)da[i].low);
    vpi_printf("\n");
    CHECK(vpi_chk_error(NULL) == 0, tag);
}

static void put(vpiHandle h, int n, int a, int b, int c) {
    setup(n, vpiSimTime);
    da[0].low = a; da[1].low = b; da[2].low = c;
    vpi_put_delays(h, &dl);
}

static void refused(const char *what) {
    s_vpi_error_info e;
    if (vpi_chk_error(&e) != vpiError)
        vpi_printf("FAIL: %s was not refused\n", what);
}

void delays_probe(void) {
    vpiHandle y1 = H("top.y1"), y2 = H("top.y2"), y3 = H("top.y3");

    /* The gate / continuous assignment delays, on the nets they drive. */
    show("y1", y1, 3);
    show("y2", y2, 3);
    show("y3", y3, 1);
    setup(3, vpiScaledRealTime);
    vpi_get_delays(y2, &dl);
    vpi_printf("CHK|y2_ns|%g|%g|%g\n", da[0].real, da[1].real, da[2].real);
    setup(2, vpiSimTime);
    dl.mtm_flag = 1; dl.pulsere_flag = 1;
    vpi_get_delays(y1, &dl);
    vpi_printf("CHK|y1_mtm_pulse");
    for (int i = 0; i < 18; i++)
        vpi_printf("|%d", (int)da[i].low);
    vpi_printf("\n");

    /* Writes change what the simulation uses from now on. */
    put(y1, 2, 1, 9, 0);
    CHECK(vpi_chk_error(NULL) == 0, "put y1");
    setup(3, vpiSimTime);
    dl.append_flag = 1;
    da[0].low = 1; da[1].low = 1; da[2].low = 1;
    vpi_put_delays(y2, &dl);
    CHECK(vpi_chk_error(NULL) == 0, "append to y2");
    show("y2_after", y2, 3);
    setup(1, vpiScaledRealTime);
    da[0].real = 2.0;
    vpi_put_delays(y3, &dl);
    CHECK(vpi_chk_error(NULL) == 0, "put y3 (a zero-delay gate)");

    /* Refusals write nothing. */
    put(y1, 4, 1, 1, 1);
    refused("four delays on a net");
    put(y1, 2, 0, 3, 0);
    refused("a zero rise delay with a non-zero fall delay");
    put(H("top.vy"), 1, 5, 0, 0);
    refused("a delay on a compiled vector driver");
    setup(1, vpiSimTime);
    dl.pulsere_flag = 1;
    da[0].low = 2; da[1].low = 1; da[2].low = 2;
    vpi_put_delays(y1, &dl);
    refused("pulse limits that differ from the delay");
    setup(1, vpiSuppressTime);
    vpi_get_delays(y1, &dl);
    refused("vpiSuppressTime");
    setup(1, vpiSimTime);
    vpi_get_delays(H("top.uc"), &dl);
    refused("a module has no delays");
    show("y1_after", y1, 2);

    /* Module paths. */
    vpiHandle it = vpi_iterate(vpiModPath, H("top.up")), p = NULL, x;
    int paths = 0;
    while (it && (x = vpi_scan(it)) != NULL) {
        paths++;
        p = x;
    }
    CHECK(paths == 1, "one module path in up");
    CHECK(p && vpi_get(vpiType, p) == vpiModPath, "vpiModPath object");
    /* A module path has no name of its own; its output terminal names q. */
    vpiHandle outs = p ? vpi_iterate(vpiModPathOut, p) : NULL;
    vpiHandle oterm = outs ? vpi_scan(outs) : NULL;
    vpiHandle onet = oterm ? vpi_handle(vpiExpr, oterm) : NULL;
    const char *oname = onet ? vpi_get_str(vpiFullName, onet) : NULL;
    CHECK(oname && strcmp(oname, "top.up.q") == 0, "its output net");
    if (outs && oterm)
        vpi_free_object(outs);
    show("path2", p, 2);
    show("path12", p, 12);
    put(p, 4, 1, 1, 1);
    refused("four delays on a module path");
    put(p, 2, 2, 3, 0);
    CHECK(vpi_chk_error(NULL) == 0, "put the path delays");
    show("path2_after", p, 2);
    CHECK(vpi_iterate(vpiModPath, H("top")) == NULL, "no module paths in top");

    /* Timing checks. */
    it = vpi_iterate(vpiTchk, H("top.uc"));
    vpiHandle sh = vpi_scan(it), wd = vpi_scan(it);
    CHECK(vpi_scan(it) == NULL, "two timing checks in uc");
    CHECK(vpi_get(vpiType, sh) == vpiTchk, "vpiTchk object");
    vpi_printf("CHK|tchk_types|%d|%d\n", vpi_get(vpiTchkType, sh), vpi_get(vpiTchkType, wd));
    show("setuphold", sh, 2);
    show("width", wd, 1);
    setup(2, vpiSimTime);
    vpi_get_delays(wd, &dl);
    refused("two limits of a one-limit check");
    setup(2, vpiSimTime);
    dl.pulsere_flag = 1;
    vpi_get_delays(sh, &dl);
    refused("pulse limits of a timing check");
    put(sh, 2, 1, 4, 0);
    CHECK(vpi_chk_error(NULL) == 0, "put the $setuphold limits");
    show("setuphold_after", sh, 2);

    /* The interconnect between two ports. */
    vpiHandle o = H("top.ud.o"), i = H("top.ur.i");
    vpiHandle im = vpi_handle_multi(vpiInterModPath, o, i);
    CHECK(im && vpi_get(vpiType, im) == vpiInterModPath, "vpiInterModPath object");
    vpiHandle im2 = vpi_handle_multi(vpiInterModPath, i, o);
    CHECK(im2 && vpi_compare_objects(im, im2), "either order names one path");
    show("intermod", im, 2);
    put(im, 2, 4, 4, 0);
    CHECK(vpi_chk_error(NULL) == 0, "put the interconnect delay");
    show("intermod_after", im, 3);
    setup(1, vpiSimTime);
    vpi_get_delays(im, &dl);
    refused("one delay on an intermodule path");
    CHECK(vpi_handle_multi(vpiModule, o, i) == NULL, "vpiModule is not a many-to-one relation");
    refused("vpi_handle_multi(vpiModule)");
    CHECK(vpi_handle_multi(vpiInterModPath, i, i) == NULL, "two input ports");
    refused("vpi_handle_multi on two input ports");
}
"#;

const DELAYS_SV: &str = r#"
`timescale 1ns/1ns
module pathmod(input d, output q);
  buf (q, d);
  specify
    (d => q) = (4, 7);
  endspecify
endmodule
module chk(input clk, input d);
  reg notifier;
  specify
    $setuphold(posedge clk, d, 3, 2, notifier);
    $width(posedge clk, 5);
  endspecify
endmodule
module drv(output o);
  reg r = 0;
  assign o = r;
  initial #40 r = 1;
endmodule
module rcv(input i);
  always @(i) $display("E i=%b t=%0t", i, $time);
endmodule
module top;
  reg a = 0, b = 1, d = 0, clk = 0, dd = 0;
  wire y1, y2, y3, q, link;
  wire [3:0] vy;
  and #(3, 5) g1 (y1, a, b);
  assign #(2, 4, 6) y2 = a;
  buf g3 (y3, a);
  assign vy = {4{a}} & 4'b1010;
  pathmod up(.d(d), .q(q));
  chk uc(.clk(clk), .d(dd));
  drv ud(.o(link));
  rcv ur(.i(link));
  always @(y1) $display("E y1=%b t=%0t", y1, $time);
  always @(y2) $display("E y2=%b t=%0t", y2, $time);
  always @(y3) $display("E y3=%b t=%0t", y3, $time);
  always @(q) $display("E q=%b t=%0t", q, $time);
  import "DPI-C" context function void delays_probe();
  initial begin
    #20 delays_probe();
    #10 a = 1;
    #20 a = 0;
    #20 d = 1;
    #20 d = 0;
    // $setuphold, now (1, 4): d 2 before the edge is outside the setup
    // limit; d 2 after the next edge is inside the hold limit.
    #10 dd = 1;
    #2 clk = 1;
    #8 clk = 0;
    #10 clk = 1;
    #2 dd = 0;
    #20 $finish;
  end
endmodule
"#;

#[test]
fn vpi_delays_and_intermodule_paths() {
    let d = scratch("vpi_delays");
    let lib = shared_lib(&d, "delays", DELAYS_C);
    let text = run_status(&d, "--dpi-lib", &lib, DELAYS_SV, &[], false);
    let _ = std::fs::remove_dir_all(&d);
    expect(
        &text,
        &[
            // rise, fall, turn-off (the smaller of rise and fall when only
            // two were given).
            "CHK|y1|3|5|3",
            "CHK|y2|2|4|6",
            "CHK|y3|0",
            "CHK|y2_ns|2|4|6",
            // mtm + pulsere: 9 slots per delay, all the value in use.
            "CHK|y1_mtm_pulse|3|3|3|3|3|3|3|3|3|5|5|5|5|5|5|5|5|5",
            "CHK|y2_after|3|5|7",
            "CHK|y1_after|1|9",
            "CHK|path2|4|7",
            "CHK|path12|4|7|4|4|7|7|4|4|7|7|7|4",
            "CHK|path2_after|2|3",
            "CHK|tchk_types|8|4",
            "CHK|setuphold|3|2",
            "CHK|width|5",
            "CHK|setuphold_after|1|4",
            "CHK|intermod|0|0",
            "CHK|intermod_after|4|4|4",
            // The new delays at work: y1 (1, 9), y2 (3, 5), y3 2, the path
            // (2, 3), the interconnect 4.
            "E y1=1 t=31",
            "E y1=0 t=59",
            "E y2=1 t=33",
            "E y2=0 t=55",
            "E y3=1 t=32",
            "E y3=0 t=52",
            "E q=1 t=72",
            "E q=0 t=93",
            "E i=1 t=44",
            "violation in top.uc at time 122",
        ],
    );
    assert!(
        !text.contains("at time 102"),
        "the old setup limit still applied:\n{text}"
    );
}
