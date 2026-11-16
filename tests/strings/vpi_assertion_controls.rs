//! IEEE 1800-2017 Clause 39 / Clause 40: assertion handles by name and
//! filtered iteration (`vpiAssume`, `vpiConcurrentAssertion`, `vpiPropertyInst`),
//! `vpiAssertionReset`, `vpiAssertionSysOn`/`Off`/`Reset`/`End`,
//! `vpiAssertionEnableStep`/`DisableStep`, `cbAssertionVacuousSuccess`,
//! `cbAssertionDisabledEvaluation`, and interoperability with `$assertoff` /
//! `$assertkill`.

use std::path::{Path, PathBuf};
use std::process::Command;

/// §39.5.1: system kill disables new attempts but retains callbacks for SysOn.
#[test]
fn vpi_assertion_system_kill_preserves_callbacks() {
    let c = r#"
static PLI_INT32 t_setup(PLI_BYTE8 *ud) {
    (void)ud;
    vpiHandle a = vpi_handle_by_name("top.A", NULL);
    PLI_INT32 rs[] = {cbAssertionStart, cbAssertionKill, cbAssertionFailure};
    for (unsigned i=0; i<sizeof rs/sizeof rs[0]; i++)
        CHECK(vpi_register_assertion_cb(a, rs[i], on_cb, NULL) != NULL, "register");
    return 0;
}
static PLI_INT32 t_kill(PLI_BYTE8 *ud) {
    (void)ud;
    CHECK(vpi_control(vpiAssertionSysKill, NULL) == 1, "system kill");
    return 0;
}
static PLI_INT32 t_resume(PLI_BYTE8 *ud) {
    (void)ud;
    CHECK(vpi_control(vpiAssertionSysOn, NULL) == 1, "system on");
    vpi_printf("ERRORS=%d\n", errors);
    return 0;
}
static void reg_all(void) {
    reg_task("$t_setup", t_setup);
    reg_task("$t_kill", t_kill);
    reg_task("$t_resume", t_resume);
}
void (*vlog_startup_routines[])(void) = { reg_all, 0 };
"#;
    let sv = r#"
module top;
 timeunit 1ns; timeprecision 1ns;
 bit clk=0; bit b=1,c=0;
 always #5 clk=~clk;
 A: assert property (@(posedge clk) b |-> ##2 c);
 initial begin $t_setup; #12 $t_kill; #10 $t_resume; #26 $finish; end
endmodule
"#;
    let log = run_vpi_common("system_kill", c, sv);
    assert!(log.contains("ERRORS=0"), "{log}");
    assert!(log.contains("CB A KILL t=12 start=5"), "{log}");
    assert!(!log.contains("CB A START t=15"), "{log}");
    assert!(!log.contains("CB A FAILURE t=25 start=5"), "{log}");
    assert!(log.contains("CB A START t=25 start=25"), "{log}");
    assert!(log.contains("CB A FAILURE t=45 start=25"), "{log}");
}

/// §39.5.2: enabling one attempt must not enable overlapping attempts.
#[test]
fn vpi_assertion_step_selects_attempt() {
    let c = r#"static PLI_INT32 t_setup(PLI_BYTE8 *ud) {
    (void)ud;
    vpiHandle a = vpi_handle_by_name("top.A", NULL);
    s_vpi_time t = {vpiSimTime, 0, 5, 0.0};
    CHECK(vpi_register_assertion_cb(a, cbAssertionStepSuccess, on_cb, NULL) != NULL, "register");
    CHECK(vpi_control(vpiAssertionEnableStep, a, &t, vpiAssertionClockSteps) == 1, "enable");
    vpi_printf("ERRORS=%d\n", errors);
    return 0;
}
static void reg_all(void) { reg_task("$t_setup", t_setup); }
void (*vlog_startup_routines[])(void) = { reg_all, 0 };
"#;
    let sv = r#"
module top;
 timeunit 1ns; timeprecision 1ns;
 bit clk=0; bit b=1,c=0;
 always #5 clk=~clk;
 A: assert property (@(posedge clk) b |-> ##3 c);
 initial begin $t_setup; #28 $finish; end
endmodule
"#;
    let log = run_vpi_common("step_attempt", c, sv);
    assert!(log.contains("ERRORS=0"), "{log}");
    assert!(log.contains("CB A STEP_OK t=15 start=5"), "{log}");
    assert!(
        !log.lines()
            .any(|l| l.contains("STEP_OK") && l.contains("start=15")),
        "{log}"
    );
}

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("xezim_vpiassert_{}_{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn run_vpi(tag: &str, c: &str, sv: &str) -> String {
    let dir = scratch(tag);
    let c_path = dir.join("test.c");
    let so_path = dir.join("test.so");
    let sv_path = dir.join("top.sv");
    std::fs::write(&c_path, c).unwrap();
    std::fs::write(&sv_path, sv).unwrap();
    let include = Path::new(env!("CARGO_MANIFEST_DIR")).join("include");
    let ok = Command::new(std::env::var_os("CC").unwrap_or_else(|| "clang".into()))
        .args(["-shared", "-fPIC", "-Wall", "-I"])
        .arg(&include)
        .arg(&c_path)
        .arg("-o")
        .arg(&so_path)
        .status()
        .expect("failed to launch cc")
        .success();
    assert!(ok, "cc failed for {}", tag);
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .arg("--vpi-lib")
        .arg(&so_path)
        .args(["--no-cache", "-s", "top"])
        .arg(&sv_path)
        .current_dir(&dir)
        .output()
        .expect("failed to run xezim");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert!(out.status.success(), "{tag} failed:\n{text}");
    text
}

const CTL_C: &str = r#"
#include <stdio.h>
#include <string.h>
#include "vpi_user.h"
#include "sv_vpi_user.h"

static int errors = 0;
static vpiHandle a1 = NULL;
static vpiHandle c1 = NULL;

#define CHECK(cond, msg)                                                    \
    do {                                                                    \
        if (!(cond)) {                                                      \
            vpi_printf("FAIL: %s\n", (msg));                                \
            errors++;                                                       \
        }                                                                   \
    } while (0)

static const char *rname(PLI_INT32 r) {
    switch (r) {
    case cbAssertionStart: return "START";
    case cbAssertionSuccess: return "SUCCESS";
    case cbAssertionFailure: return "FAILURE";
    case cbAssertionStepSuccess: return "STEP_OK";
    case cbAssertionStepFailure: return "STEP_FAIL";
    case cbAssertionDisable: return "DISABLE";
    case cbAssertionEnable: return "ENABLE";
    case cbAssertionReset: return "RESET";
    case cbAssertionKill: return "KILL";
    case cbAssertionVacuousSuccess: return "VACUOUS";
    case cbAssertionDisabledEvaluation: return "DISABLED_EVAL";
    default: return "OTHER";
    }
}

static PLI_INT32 on_cb(PLI_INT32 reason, p_vpi_time cb_time,
                       vpiHandle assertion, p_vpi_attempt_info info,
                       PLI_BYTE8 *user_data) {
    (void)user_data;
    vpi_printf("CB %s %s t=%u start=%u\n", vpi_get_str(vpiName, assertion),
               rname(reason), cb_time ? cb_time->low : 0u,
               info ? info->attemptStartTime.low : 0u);
    return 0;
}

static int count_rel(PLI_INT32 rel, vpiHandle scope) {
    int n = 0;
    vpiHandle it = vpi_iterate(rel, scope), o;
    while (it && (o = vpi_scan(it)) != NULL) {
        n++;
        vpi_free_object(o);
    }
    return n;
}

static PLI_INT32 va2_setup(PLI_BYTE8 *ud) {
    (void)ud;
    vpiHandle top = vpi_handle_by_name("top", NULL);
    vpiHandle uif = vpi_handle_by_name("top.u_if", NULL);
    a1 = vpi_handle_by_name("top.A1", NULL);
    c1 = vpi_handle_by_name("top.C1", NULL);
    vpiHandle m1 = vpi_handle_by_name("top.u_if.M1", NULL);
    CHECK(top && uif && a1 && c1 && m1, "handle_by_name resolves assertions");
    CHECK(vpi_get(vpiType, a1) == vpiAssert, "A1 is vpiAssert");
    CHECK(strcmp(vpi_get_str(vpiType, a1), "vpiAssert") == 0, "A1 vpiType str");
    CHECK(vpi_get(vpiType, m1) == vpiAssume, "M1 is vpiAssume");
    CHECK(strcmp(vpi_get_str(vpiType, m1), "vpiAssume") == 0, "M1 vpiType str");
    CHECK(count_rel(vpiConcurrentAssertion, top) == 2, "2 concurrent assertions in top");
    CHECK(count_rel(vpiAssert, top) == 1, "1 vpiAssert in top");
    CHECK(count_rel(vpiCover, top) == 1, "1 vpiCover in top");
    CHECK(count_rel(vpiPropertyInst, top) == 1, "1 vpiPropertyInst in top");
    CHECK(count_rel(vpiAssume, uif) == 1, "1 vpiAssume in u_if");

    PLI_INT32 reasons[] = {
        cbAssertionStart, cbAssertionSuccess, cbAssertionFailure,
        cbAssertionStepSuccess, cbAssertionStepFailure,
        cbAssertionDisable, cbAssertionEnable, cbAssertionReset,
        cbAssertionKill, cbAssertionVacuousSuccess, cbAssertionDisabledEvaluation
    };
    for (unsigned i = 0; i < sizeof reasons / sizeof reasons[0]; i++)
        CHECK(vpi_register_assertion_cb(a1, reasons[i], on_cb, NULL) != NULL, "reg a1");
    CHECK(vpi_register_assertion_cb(m1, cbAssertionDisable, on_cb, NULL) != NULL, "reg m1");
    CHECK(vpi_register_assertion_cb(m1, cbAssertionEnable, on_cb, NULL) != NULL, "reg m1");
    return 0;
}

static PLI_INT32 va2_ctl(PLI_BYTE8 *ud) {
    (void)ud;
    vpiHandle call = vpi_handle(vpiSysTfCall, NULL);
    vpiHandle it = vpi_iterate(vpiArgument, call);
    s_vpi_value v;
    v.format = vpiIntVal;
    vpi_get_value(vpi_scan(it), &v);
    vpi_free_object(it);
    PLI_INT32 op = v.value.integer;
    if (op >= vpiAssertionSysOn && op <= vpiAssertionSysReset)
        CHECK(vpi_control(op) == 1, "sys vpi_control");
    else
        CHECK(vpi_control(op, a1) == 1, "handle vpi_control");
    return 0;
}

static PLI_INT32 va2_enable_step(PLI_BYTE8 *ud) {
    (void)ud;
    s_vpi_time t = {vpiSimTime, 0, 15, 0.0};
    CHECK(vpi_control(vpiAssertionEnableStep, a1, &t, 999) == 0, "bad step rejected");
    vpi_chk_error(NULL);
    CHECK(vpi_control(vpiAssertionEnableStep, a1, &t, vpiAssertionClockSteps) == 1, "enable step");
    t.low = 45;
    CHECK(vpi_control(vpiAssertionEnableStep, a1, &t, vpiAssertionClockSteps) == 1, "enable future attempt");
    return 0;
}

static PLI_INT32 va2_disable_step(PLI_BYTE8 *ud) {
    (void)ud;
    s_vpi_time t = {vpiSimTime, 0, 45, 0.0};
    CHECK(vpi_control(vpiAssertionDisableStep, a1, &t) == 1, "disable step");
    return 0;
}

static PLI_INT32 va2_check(PLI_BYTE8 *ud) {
    (void)ud;
    vpi_printf("ATTEMPT_COV=%d SUCCESS_COV=%d\n",
               (int)vpi_get(vpiAssertAttemptCovered, c1),
               (int)vpi_get(vpiAssertSuccessCovered, c1));
    vpi_printf("VA2_ERRORS=%d\n", errors);
    return 0;
}

static void va2_reg(void) {
    s_vpi_systf_data d;
    memset(&d, 0, sizeof d);
    d.type = vpiSysTask;
    d.tfname = "$va2_setup"; d.calltf = va2_setup; vpi_register_systf(&d);
    d.tfname = "$va2_ctl"; d.calltf = va2_ctl; vpi_register_systf(&d);
    d.tfname = "$va2_enable_step"; d.calltf = va2_enable_step; vpi_register_systf(&d);
    d.tfname = "$va2_disable_step"; d.calltf = va2_disable_step; vpi_register_systf(&d);
    d.tfname = "$va2_check"; d.calltf = va2_check; vpi_register_systf(&d);
}

void (*vlog_startup_routines[])(void) = { va2_reg, 0 };
"#;

const CTL_SV: &str = r#"
`timescale 1ns/1ns
interface chk_if(input logic clk);
  logic x = 0, y = 0;
  M1: assume property (@(posedge clk) x |-> y);
endinterface

module top;
  logic clk = 0;
  logic a = 1, b = 0, c = 0, rst = 0;
  always #5 clk = ~clk;

  chk_if u_if(.clk(clk));

  property p_imp;
    @(posedge clk) disable iff (rst) b |-> ##2 c;
  endproperty
  A1: assert property (p_imp);
  C1: cover property (@(posedge clk) a);

  initial begin
    $va2_setup;
    #10 b = 1;
    #10 b = 0;
    #2 $va2_enable_step;
    #5 $va2_ctl(622); // vpiAssertionReset at t=27
    #10 b = 1; // t=37
    #10 b = 0; // t=47
    #1 $va2_disable_step; // t=48
    #10 rst = 1; // t=58
    #10 rst = 0; // t=68
    #2 $assertoff(0, top.A1); // t=70
    #2 $va2_ctl(621); // vpiAssertionEnable at t=72
    #2 b = 1; // t=74
    #6 b = 0; $assertkill(0, top.A1); // t=80
    #2 $va2_ctl(627); // vpiAssertionSysOn at t=82
    #2 b = 1; // t=84
    #6 b = 0; $va2_ctl(628); // vpiAssertionSysOff at t=90
    #2 $va2_ctl(630); // vpiAssertionSysReset at t=92
    #2 b = 1; $va2_ctl(627); // vpiAssertionSysOn at t=94
    #6 b = 0; $va2_ctl(629); // vpiAssertionSysEnd at t=100
    #10 $va2_check;
    $display("RESULT: PASSED");
    $finish;
  end
endmodule
"#;

#[test]
fn vpi_assertion_controls_and_step_test() {
    let log = run_vpi("controls", CTL_C, CTL_SV);
    assert!(log.contains("VA2_ERRORS=0"), "C checks failed:\n{log}");
    assert!(log.contains("RESULT: PASSED"), "missing PASSED:\n{log}");
    let cbs: Vec<&str> = log.lines().filter(|l| l.starts_with("CB ")).collect();
    assert_eq!(
        cbs,
        [
            "CB A1 START t=5 start=5",
            "CB A1 VACUOUS t=5 start=5",
            "CB A1 START t=15 start=15",
            "CB A1 START t=25 start=25",
            "CB A1 STEP_OK t=25 start=15",
            "CB A1 VACUOUS t=25 start=25",
            "CB A1 RESET t=27 start=27",
            "CB A1 START t=35 start=35",
            "CB A1 VACUOUS t=35 start=35",
            "CB A1 START t=45 start=45",
            "CB A1 STEP_OK t=45 start=45",
            "CB A1 START t=55 start=55",
            "CB A1 VACUOUS t=55 start=55",
            "CB A1 DISABLED_EVAL t=58 start=45",
            "CB A1 DISABLE t=70 start=70",
            "CB A1 ENABLE t=72 start=72",
            "CB A1 START t=75 start=75",
            "CB A1 KILL t=80 start=75",
            "CB A1 DISABLE t=80 start=80",
            "CB A1 ENABLE t=82 start=82",
            "CB A1 START t=85 start=85",
            "CB A1 DISABLE t=90 start=90",
            "CB M1 DISABLE t=90 start=90",
            "CB A1 RESET t=92 start=92",
            "CB A1 ENABLE t=94 start=94",
            "CB M1 ENABLE t=94 start=94",
            "CB A1 START t=95 start=95",
            "CB A1 KILL t=100 start=95",
            "CB A1 DISABLE t=100 start=100",
            "CB M1 DISABLE t=100 start=100",
        ],
        "unexpected CB transcript:\n{log}"
    );
    assert_eq!(
        log.matches("ATTEMPT_COV=10 SUCCESS_COV=10").count(),
        1,
        "unexpected coverage count:\n{log}"
    );
}

const COMMON_C: &str = r#"
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

static const char *rname(PLI_INT32 r) {
    switch (r) {
    case cbAssertionStart: return "START";
    case cbAssertionSuccess: return "SUCCESS";
    case cbAssertionFailure: return "FAILURE";
    case cbAssertionStepSuccess: return "STEP_OK";
    case cbAssertionStepFailure: return "STEP_FAIL";
    case cbAssertionDisable: return "DISABLE";
    case cbAssertionEnable: return "ENABLE";
    case cbAssertionReset: return "RESET";
    case cbAssertionKill: return "KILL";
    case cbAssertionVacuousSuccess: return "VACUOUS";
    case cbAssertionDisabledEvaluation: return "DISABLED_EVAL";
    default: return "OTHER";
    }
}

static inline PLI_INT32 on_cb(PLI_INT32 reason, p_vpi_time cb_time,
                              vpiHandle assertion, p_vpi_attempt_info info,
                              PLI_BYTE8 *user_data) {
    (void)user_data;
    CHECK(cb_time != NULL, "cb_time is non-NULL");
    CHECK(info != NULL, "info is non-NULL");
    vpi_printf("CB %s %s t=%u start=%u\n", vpi_get_str(vpiName, assertion),
               rname(reason), cb_time ? cb_time->low : 0u,
               info ? info->attemptStartTime.low : 0u);
    return 0;
}

static inline int count_rel(PLI_INT32 rel, vpiHandle scope) {
    int n = 0;
    vpiHandle it = vpi_iterate(rel, scope), o;
    while (it && (o = vpi_scan(it)) != NULL) {
        n++;
        vpi_free_object(o);
    }
    return n;
}

static void reg_task(const char *name, PLI_INT32 (*fn)(PLI_BYTE8 *)) {
    s_vpi_systf_data d;
    memset(&d, 0, sizeof d);
    d.type = vpiSysTask;
    d.tfname = (PLI_BYTE8 *)name;
    d.calltf = fn;
    vpi_register_systf(&d);
}
"#;

fn run_vpi_common(tag: &str, body_c: &str, sv: &str) -> String {
    let c = format!("{COMMON_C}\n{body_c}");
    run_vpi(tag, &c, sv)
}

/// IEEE 1800-2017 Clause 39.5 / §20.12: `vpiAssertionSysOff` opens a quiet
/// window across clock edges (no `START` inside, `START` on the first edge
/// after `vpiAssertionSysOn`), and `vpiAssertionSysEnd` removes callbacks so a
/// subsequent `$asserton` resumes assertion attempts without firing callbacks.
#[test]
fn vpi_assertion_sys_end_and_sys_off_windows() {
    let c = r#"
static vpiHandle a_h = NULL, c_h = NULL;
static PLI_INT32 t_setup(PLI_BYTE8 *ud) {
    (void)ud;
    a_h = vpi_handle_by_name("top.A", NULL);
    c_h = vpi_handle_by_name("top.C", NULL);
    CHECK(a_h && c_h, "handles resolved");
    PLI_INT32 rs[] = {cbAssertionStart, cbAssertionSuccess, cbAssertionDisable,
                      cbAssertionEnable, cbAssertionKill};
    for (unsigned i = 0; i < sizeof rs / sizeof rs[0]; i++)
        CHECK(vpi_register_assertion_cb(a_h, rs[i], on_cb, NULL) != NULL, "reg");
    return 0;
}
static PLI_INT32 t_sys(PLI_BYTE8 *ud) {
    (void)ud;
    vpiHandle call = vpi_handle(vpiSysTfCall, NULL);
    vpiHandle it = vpi_iterate(vpiArgument, call);
    s_vpi_value v;
    v.format = vpiIntVal;
    vpi_get_value(vpi_scan(it), &v);
    vpi_free_object(it);
    CHECK(vpi_control(v.value.integer) == 1, "sys control");
    return 0;
}
static PLI_INT32 t_check(PLI_BYTE8 *ud) {
    (void)ud;
    vpi_printf("COVER_SUCCESS=%d\n", (int)vpi_get(vpiAssertSuccessCovered, c_h));
    vpi_printf("ERRORS=%d\n", errors);
    return 0;
}
static void reg_all(void) {
    reg_task("$t_setup", t_setup);
    reg_task("$t_sys", t_sys);
    reg_task("$t_check", t_check);
}
void (*vlog_startup_routines[])(void) = { reg_all, 0 };
"#;
    let sv = r#"
`timescale 1ns/1ns
module top;
  logic clk = 0;
  logic a = 1;
  always #5 clk = ~clk;
  A: assert property (@(posedge clk) a) $display("A_ACTION t=%0t", $time);
  C: cover property (@(posedge clk) a);
  initial begin
    $t_setup;
    #10 $t_sys(628); // t=10: vpiAssertionSysOff
    #20 $t_sys(627); // t=30: vpiAssertionSysOn (edges 15, 25 inside window)
    #10 $t_sys(629); // t=40: vpiAssertionSysEnd (after edge 35)
    #2 $asserton(0, top); // t=42: re-enables assertions after SysEnd
    #20; // t=62: edges 45 and 55 pass with A attempting
    $t_check;
    $display("RESULT: PASSED");
    $finish;
  end
endmodule
"#;
    let log = run_vpi_common("sys_windows", c, sv);
    assert!(log.contains("ERRORS=0"), "C checks failed:\n{log}");
    assert!(log.contains("RESULT: PASSED"), "missing PASSED:\n{log}");
    let cbs: Vec<&str> = log.lines().filter(|l| l.starts_with("CB ")).collect();
    assert_eq!(
        cbs,
        [
            "CB A START t=5 start=5",
            "CB A SUCCESS t=5 start=5",
            "CB A DISABLE t=10 start=10",
            "CB A ENABLE t=30 start=30",
            "CB A START t=35 start=35",
            "CB A SUCCESS t=35 start=35",
            "CB A DISABLE t=40 start=40",
        ],
        "unexpected CB transcript:\n{log}"
    );
    assert_eq!(log.matches("A_ACTION t=5\n").count(), 1, "{log}");
    assert!(!log.contains("A_ACTION t=15"), "{log}");
    assert!(!log.contains("A_ACTION t=25"), "{log}");
    assert_eq!(log.matches("A_ACTION t=35\n").count(), 1, "{log}");
    assert_eq!(log.matches("A_ACTION t=45\n").count(), 1, "{log}");
    assert_eq!(log.matches("A_ACTION t=55\n").count(), 1, "{log}");
    assert_eq!(log.matches("COVER_SUCCESS=4").count(), 1, "{log}");
}

/// IEEE 1800-2017 Clause 39.5: `vpiAssertionKill` with a specific attempt start
/// time kills only that attempt while another in-flight attempt resolves
/// normally.
#[test]
fn vpi_assertion_kill_by_time() {
    let c = r#"
static vpiHandle a_h = NULL;
static PLI_INT32 t_setup(PLI_BYTE8 *ud) {
    (void)ud;
    a_h = vpi_handle_by_name("top.A", NULL);
    CHECK(a_h != NULL, "A handle");
    PLI_INT32 rs[] = {cbAssertionStart, cbAssertionSuccess,
                      cbAssertionFailure, cbAssertionKill};
    for (unsigned i = 0; i < sizeof rs / sizeof rs[0]; i++)
        CHECK(vpi_register_assertion_cb(a_h, rs[i], on_cb, NULL) != NULL, "reg");
    return 0;
}
static PLI_INT32 t_kill_65(PLI_BYTE8 *ud) {
    (void)ud;
    s_vpi_time t = {vpiSimTime, 0, 65, 0.0};
    CHECK(vpi_control(vpiAssertionKill, a_h, &t) == 1, "kill 65");
    return 0;
}
static PLI_INT32 t_check(PLI_BYTE8 *ud) {
    (void)ud;
    vpi_printf("ERRORS=%d\n", errors);
    return 0;
}
static void reg_all(void) {
    reg_task("$t_setup", t_setup);
    reg_task("$t_kill_65", t_kill_65);
    reg_task("$t_check", t_check);
}
void (*vlog_startup_routines[])(void) = { reg_all, 0 };
"#;
    let sv = r#"
`timescale 1ns/1ns
module top;
  logic clk = 0;
  logic b = 0, c = 0;
  always #5 clk = ~clk;
  A: assert property (@(posedge clk) b |-> ##2 c);
  initial begin
    $t_setup;
    #60 b = 1; // t=60: attempts start at 65 and 75
    #20 b = 0; // t=80: both in flight
    $t_kill_65;
    #10 c = 1; // t=90: attempt 75 succeeds at 95
    #8;
    $t_check;
    $display("RESULT: PASSED");
    $finish;
  end
endmodule
"#;
    let log = run_vpi_common("kill_time", c, sv);
    assert!(log.contains("ERRORS=0"), "C checks failed:\n{log}");
    assert!(log.contains("RESULT: PASSED"), "missing PASSED:\n{log}");
    let cbs: Vec<&str> = log.lines().filter(|l| l.starts_with("CB ")).collect();
    assert_eq!(
        cbs,
        [
            "CB A START t=5 start=5",
            "CB A START t=15 start=15",
            "CB A START t=25 start=25",
            "CB A START t=35 start=35",
            "CB A START t=45 start=45",
            "CB A START t=55 start=55",
            "CB A START t=65 start=65",
            "CB A START t=75 start=75",
            "CB A KILL t=80 start=65",
            "CB A START t=85 start=85",
            "CB A START t=95 start=95",
            "CB A SUCCESS t=95 start=75",
        ],
        "unexpected CB transcript:\n{log}"
    );
}

/// IEEE 1800-2017 Clause 39.4.2 / §16.12.2: `cbAssertionStepFailure` fires
/// immediately before `cbAssertionFailure` on a failing multi-cycle attempt
/// and when a strong property fails at `$finish`.
#[test]
fn vpi_assertion_step_failure_and_finish() {
    let c = r#"
static PLI_INT32 t_setup(PLI_BYTE8 *ud) {
    (void)ud;
    vpiHandle a = vpi_handle_by_name("top.A", NULL);
    vpiHandle s = vpi_handle_by_name("top.S", NULL);
    CHECK(a && s, "handles");
    s_vpi_time t = {vpiSimTime, 0, 0, 0.0};
    PLI_INT32 rs[] = {cbAssertionStart, cbAssertionStepSuccess,
                      cbAssertionStepFailure, cbAssertionFailure};
    for (unsigned i = 0; i < sizeof rs / sizeof rs[0]; i++) {
        CHECK(vpi_register_assertion_cb(a, rs[i], on_cb, NULL) != NULL, "reg a");
        CHECK(vpi_register_assertion_cb(s, rs[i], on_cb, NULL) != NULL, "reg s");
    }
    t.low = 5;
    CHECK(vpi_control(vpiAssertionEnableStep, a, &t, vpiAssertionClockSteps) == 1, "step a");
    t.low = 35;
    CHECK(vpi_control(vpiAssertionEnableStep, s, &t, vpiAssertionClockSteps) == 1, "step s");
    return 0;
}
static PLI_INT32 t_check(PLI_BYTE8 *ud) {
    (void)ud;
    vpi_printf("ERRORS=%d\n", errors);
    return 0;
}
static void reg_all(void) {
    reg_task("$t_setup", t_setup);
    reg_task("$t_check", t_check);
}
void (*vlog_startup_routines[])(void) = { reg_all, 0 };
"#;
    let sv = r#"
`timescale 1ns/1ns
module top;
  logic clk = 0;
  logic b = 0, c = 0, sb = 0, sc = 0;
  always #5 clk = ~clk;
  A: assert property (@(posedge clk) b |-> ##2 c) else $display("A_FAIL t=%0t", $time);
  S: assert property (@(posedge clk) sb |-> s_eventually sc) else $display("S_FAIL t=%0t", $time);
  initial begin
    $t_setup;
    b = 1;
    #10 b = 0;  // t=10
    #20 sb = 1; // t=30: S starts at t=35
    #10;        // t=40: $finish with S attempt 35 unresolved
    $t_check;
    $display("RESULT: PASSED");
    $finish;
  end
endmodule
"#;
    let log = run_vpi_common("step_fail", c, sv);
    assert!(log.contains("ERRORS=0"), "C checks failed:\n{log}");
    assert!(log.contains("RESULT: PASSED"), "missing PASSED:\n{log}");
    let cbs: Vec<&str> = log.lines().filter(|l| l.starts_with("CB ")).collect();
    assert_eq!(
        cbs,
        [
            "CB A START t=5 start=5",
            "CB A STEP_OK t=5 start=5",
            "CB S START t=5 start=5",
            "CB A START t=15 start=15",
            "CB A STEP_OK t=15 start=5",
            "CB S START t=15 start=15",
            "CB A START t=25 start=25",
            "CB A STEP_FAIL t=25 start=5",
            "CB A FAILURE t=25 start=5",
            "CB S START t=25 start=25",
            "CB A START t=35 start=35",
            "CB S START t=35 start=35",
            "CB S STEP_OK t=35 start=35",
            "CB S STEP_FAIL t=40 start=35",
            "CB S FAILURE t=40 start=35",
        ],
        "unexpected CB transcript:\n{log}"
    );
}

/// IEEE 1800-2017 Clause 40: coverage on a `cover` whose condition is false on
/// some attempts pins `vpiAssertSuccessCovered` to true samples,
/// `vpiAssertFailureCovered` to 0, and `vpiAssertAttemptCovered` to the
/// documented `pass + fail` approximation.
#[test]
fn vpi_assertion_cover_counts() {
    let c = r#"
static vpiHandle c_h = NULL;
static PLI_INT32 t_setup(PLI_BYTE8 *ud) {
    (void)ud;
    c_h = vpi_handle_by_name("top.C", NULL);
    CHECK(c_h != NULL, "cover handle");
    return 0;
}
static PLI_INT32 t_check(PLI_BYTE8 *ud) {
    (void)ud;
    vpi_printf("COV attempt=%d success=%d failure=%d\n",
               (int)vpi_get(vpiAssertAttemptCovered, c_h),
               (int)vpi_get(vpiAssertSuccessCovered, c_h),
               (int)vpi_get(vpiAssertFailureCovered, c_h));
    vpi_printf("ERRORS=%d\n", errors);
    return 0;
}
static void reg_all(void) {
    reg_task("$t_setup", t_setup);
    reg_task("$t_check", t_check);
}
void (*vlog_startup_routines[])(void) = { reg_all, 0 };
"#;
    let sv = r#"
`timescale 1ns/1ns
module top;
  logic clk = 0;
  logic a = 1;
  always #5 clk = ~clk; // posedges at 5, 15, 25, 35, 45, 55
  C: cover property (@(posedge clk) a);
  initial begin
    $t_setup;
    #20 a = 0; // t=20 -> 25 false
    #10 a = 1; // t=30 -> 35, 45 true
    #20 a = 0; // t=50 -> 55 false
    #10;
    $t_check;
    $display("RESULT: PASSED");
    $finish;
  end
endmodule
"#;
    let log = run_vpi_common("cover_counts", c, sv);
    assert!(log.contains("ERRORS=0"), "C checks failed:\n{log}");
    assert!(log.contains("RESULT: PASSED"), "missing PASSED:\n{log}");
    // Documented approximation: `sva_tally` does not tally cover failures, so
    // `vpiAssertAttemptCovered` (`pass + fail`) equals `vpiAssertSuccessCovered`.
    assert_eq!(
        log.matches("COV attempt=4 success=4 failure=0").count(),
        1,
        "unexpected cover counts:\n{log}"
    );
}

/// IEEE 1800-2017 Clause 39.3 / 39.4.2: `vpiPropertyInst` is created in child
/// modules, interfaces, generate scopes, clocked directives and for package
/// properties, never for sequence covers; callbacks on a child-scope
/// `vpiPropertyInst` fire on `Success` and never on `Disable`.
#[test]
fn vpi_assertion_property_inst_scopes() {
    let c = r#"
static PLI_INT32 t_setup(PLI_BYTE8 *ud) {
    (void)ud;
    vpiHandle top = vpi_handle_by_name("top", NULL);
    vpiHandle u_if = vpi_handle_by_name("top.u_if", NULL);
    vpiHandle u_mod = vpi_handle_by_name("top.u_mod", NULL);
    vpiHandle u_pkg = vpi_handle_by_name("top.u_pkg", NULL);
    vpiHandle g_blk = vpi_handle_by_name("top.g_blk", NULL);
    CHECK(top && u_if && u_mod && u_pkg && g_blk, "scopes resolved");
    CHECK(count_rel(vpiPropertyInst, u_if) == 1, "1 vpiPropertyInst in u_if");
    CHECK(count_rel(vpiPropertyInst, u_mod) == 1, "1 vpiPropertyInst in u_mod");
    CHECK(count_rel(vpiPropertyInst, g_blk) == 1, "1 vpiPropertyInst in g_blk");
    CHECK(count_rel(vpiPropertyInst, u_pkg) == 1, "1 vpiPropertyInst in u_pkg");
    CHECK(count_rel(vpiPropertyInst, top) == 1, "1 clocked vpiPropertyInst in top, 0 for sequence");
    vpiHandle it = vpi_iterate(vpiPropertyInst, u_mod);
    vpiHandle inst = it ? vpi_scan(it) : NULL;
    if (it) vpi_free_object(it);
    CHECK(inst != NULL, "u_mod property inst handle");
    if (inst) {
        CHECK(vpi_get(vpiType, inst) == vpiPropertyInst, "type is vpiPropertyInst");
        CHECK(vpi_register_assertion_cb(inst, cbAssertionSuccess, on_cb, NULL) != NULL, "reg success");
        CHECK(vpi_register_assertion_cb(inst, cbAssertionDisable, on_cb, NULL) != NULL, "reg disable");
    }
    return 0;
}
static PLI_INT32 t_check(PLI_BYTE8 *ud) {
    (void)ud;
    vpi_printf("ERRORS=%d\n", errors);
    return 0;
}
static void reg_all(void) {
    reg_task("$t_setup", t_setup);
    reg_task("$t_check", t_check);
}
void (*vlog_startup_routines[])(void) = { reg_all, 0 };
"#;
    let sv = r#"
`timescale 1ns/1ns
package p_pkg;
  property p_from_pkg(logic x);
    x;
  endproperty
endpackage
interface sub_if(input logic clk, input logic s);
  property p_if;
    @(posedge clk) s;
  endproperty
  A_IF: assert property (p_if);
endinterface
module sub_mod(input logic clk, input logic s);
  property p_mod;
    @(posedge clk) s;
  endproperty
  A_MOD: assert property (p_mod);
endmodule
module sub_pkg(input logic clk, input logic s);
  import p_pkg::*;
  A_PKG: assert property (@(posedge clk) p_from_pkg(s));
endmodule
module top;
  logic clk = 0;
  logic a = 1;
  always #5 clk = ~clk;
  sub_if u_if(.clk(clk), .s(a));
  sub_mod u_mod(.clk(clk), .s(a));
  sub_pkg u_pkg(.clk(clk), .s(a));
  if (1) begin : g_blk
    property p_gen;
      @(posedge clk) a;
    endproperty
    A_GEN: assert property (p_gen);
  end
  sequence s_top;
    @(posedge clk) a;
  endsequence
  C_SEQ: cover property (s_top);
  property p_top;
    a;
  endproperty
  A_CLK: assert property (@(posedge clk) p_top);
  initial begin
    $t_setup;
    #10 $assertoff(0, top.u_mod.A_MOD); // t=10
    #10 $asserton(0, top.u_mod.A_MOD);  // t=20
    #10;
    $t_check;
    $display("RESULT: PASSED");
    $finish;
  end
endmodule
"#;
    let log = run_vpi_common("prop_inst_scopes", c, sv);
    assert!(log.contains("ERRORS=0"), "C checks failed:\n{log}");
    assert!(log.contains("RESULT: PASSED"), "missing PASSED:\n{log}");
    let cbs: Vec<&str> = log.lines().filter(|l| l.starts_with("CB ")).collect();
    assert_eq!(
        cbs,
        [
            "CB p_mod SUCCESS t=5 start=5",
            "CB p_mod SUCCESS t=25 start=25",
        ],
        "unexpected CB transcript:\n{log}"
    );
}

/// IEEE 1800-2017 Clause 39.4.2 / 39.5: `cbAssertionStart` fires before the
/// attempt advances, so `vpiAssertionKill` or `vpiAssertionReset` from the
/// Start callback aborts a single-cycle attempt before its outcome or action
/// block runs.
#[test]
fn vpi_assertion_kill_from_start_callback() {
    let c = r#"
static int killed_once = 0;
static int reset_once = 0;
static PLI_INT32 on_start_kill(PLI_INT32 reason, p_vpi_time cb_time,
                               vpiHandle assertion, p_vpi_attempt_info info,
                               PLI_BYTE8 *ud) {
    on_cb(reason, cb_time, assertion, info, ud);
    if (!killed_once) {
        killed_once = 1;
        CHECK(vpi_control(vpiAssertionKill, assertion, NULL) == 1, "kill from start");
    }
    return 0;
}
static PLI_INT32 on_start_reset(PLI_INT32 reason, p_vpi_time cb_time,
                                vpiHandle assertion, p_vpi_attempt_info info,
                                PLI_BYTE8 *ud) {
    on_cb(reason, cb_time, assertion, info, ud);
    if (!reset_once) {
        reset_once = 1;
        CHECK(vpi_control(vpiAssertionReset, assertion) == 1, "reset from start");
    }
    return 0;
}
static PLI_INT32 t_setup(PLI_BYTE8 *ud) {
    (void)ud;
    vpiHandle a1 = vpi_handle_by_name("top.A1", NULL);
    vpiHandle a2 = vpi_handle_by_name("top.A2", NULL);
    CHECK(a1 && a2, "handles");
    CHECK(vpi_register_assertion_cb(a1, cbAssertionStart, on_start_kill, NULL) != NULL, "reg a1 start");
    CHECK(vpi_register_assertion_cb(a1, cbAssertionSuccess, on_cb, NULL) != NULL, "reg a1 ok");
    CHECK(vpi_register_assertion_cb(a1, cbAssertionFailure, on_cb, NULL) != NULL, "reg a1 fail");
    CHECK(vpi_register_assertion_cb(a1, cbAssertionKill, on_cb, NULL) != NULL, "reg a1 kill");
    CHECK(vpi_register_assertion_cb(a2, cbAssertionStart, on_start_reset, NULL) != NULL, "reg a2 start");
    CHECK(vpi_register_assertion_cb(a2, cbAssertionSuccess, on_cb, NULL) != NULL, "reg a2 ok");
    CHECK(vpi_register_assertion_cb(a2, cbAssertionFailure, on_cb, NULL) != NULL, "reg a2 fail");
    CHECK(vpi_register_assertion_cb(a2, cbAssertionReset, on_cb, NULL) != NULL, "reg a2 reset");
    return 0;
}
static PLI_INT32 t_check(PLI_BYTE8 *ud) {
    (void)ud;
    vpi_printf("ERRORS=%d\n", errors);
    return 0;
}
static void reg_all(void) {
    reg_task("$t_setup", t_setup);
    reg_task("$t_check", t_check);
}
void (*vlog_startup_routines[])(void) = { reg_all, 0 };
"#;
    let sv = r#"
`timescale 1ns/1ns
module top;
  logic clk = 0;
  logic x = 0;
  always #5 clk = ~clk;
  A1: assert property (@(posedge clk) x) else $display("ACTION1 t=%0t", $time);
  A2: assert property (@(posedge clk) x) else $display("ACTION2 t=%0t", $time);
  initial begin
    $t_setup;
    #20;
    $t_check;
    $display("RESULT: PASSED");
    $finish;
  end
endmodule
"#;
    let log = run_vpi_common("kill_from_start", c, sv);
    assert!(log.contains("ERRORS=0"), "C checks failed:\n{log}");
    assert!(log.contains("RESULT: PASSED"), "missing PASSED:\n{log}");
    let cbs: Vec<&str> = log.lines().filter(|l| l.starts_with("CB ")).collect();
    assert_eq!(
        cbs,
        [
            "CB A1 START t=5 start=5",
            "CB A1 KILL t=5 start=5",
            "CB A2 START t=5 start=5",
            "CB A2 RESET t=5 start=5",
            "CB A1 START t=15 start=15",
            "CB A1 FAILURE t=15 start=15",
            "CB A2 START t=15 start=15",
            "CB A2 FAILURE t=15 start=15",
        ],
        "unexpected CB transcript:\n{log}"
    );
    assert!(!log.contains("ACTION1 t=5"), "{log}");
    assert!(!log.contains("ACTION2 t=5"), "{log}");
    assert_eq!(log.matches("ACTION1 t=15").count(), 1, "{log}");
    assert_eq!(log.matches("ACTION2 t=15").count(), 1, "{log}");
}

/// IEEE 1800-2017 Clause 39.5 / §20.12: reentrant `vpi_control` from inside
/// `cbAssertionKill` or `cbAssertionDisable` must not double-fire
/// `cbAssertionDisable` or emit stale transitions for later targets.
#[test]
fn vpi_assertion_reentrant_controls() {
    let c = r#"
static vpiHandle a_h = NULL, b1_h = NULL, b2_h = NULL;
static int b1_disable_count = 0;
static PLI_INT32 on_kill_a(PLI_INT32 reason, p_vpi_time cb_time,
                           vpiHandle assertion, p_vpi_attempt_info info,
                           PLI_BYTE8 *ud) {
    on_cb(reason, cb_time, assertion, info, ud);
    CHECK(vpi_control(vpiAssertionDisable, assertion) == 1, "disable A in kill");
    return 0;
}
static PLI_INT32 on_disable_b1(PLI_INT32 reason, p_vpi_time cb_time,
                               vpiHandle assertion, p_vpi_attempt_info info,
                               PLI_BYTE8 *ud) {
    on_cb(reason, cb_time, assertion, info, ud);
    b1_disable_count++;
    if (b1_disable_count == 1)
        CHECK(vpi_control(vpiAssertionDisable, b2_h) == 1, "disable B2 in B1 disable");
    else if (b1_disable_count == 2)
        CHECK(vpi_control(vpiAssertionEnable, b2_h) == 1, "enable B2 in B1 disable");
    return 0;
}
static PLI_INT32 t_setup(PLI_BYTE8 *ud) {
    (void)ud;
    a_h = vpi_handle_by_name("top.A", NULL);
    b1_h = vpi_handle_by_name("top.B1", NULL);
    b2_h = vpi_handle_by_name("top.B2", NULL);
    CHECK(a_h && b1_h && b2_h, "handles");
    CHECK(vpi_register_assertion_cb(a_h, cbAssertionKill, on_kill_a, NULL) != NULL, "reg A kill");
    CHECK(vpi_register_assertion_cb(a_h, cbAssertionDisable, on_cb, NULL) != NULL, "reg A dis");
    CHECK(vpi_register_assertion_cb(b1_h, cbAssertionDisable, on_disable_b1, NULL) != NULL, "reg B1 dis");
    CHECK(vpi_register_assertion_cb(b1_h, cbAssertionEnable, on_cb, NULL) != NULL, "reg B1 en");
    CHECK(vpi_register_assertion_cb(b2_h, cbAssertionDisable, on_cb, NULL) != NULL, "reg B2 dis");
    CHECK(vpi_register_assertion_cb(b2_h, cbAssertionEnable, on_cb, NULL) != NULL, "reg B2 en");
    CHECK(vpi_register_assertion_cb(b2_h, cbAssertionStart, on_cb, NULL) != NULL, "reg B2 start");
    return 0;
}
static PLI_INT32 t_check(PLI_BYTE8 *ud) {
    (void)ud;
    vpi_printf("ERRORS=%d\n", errors);
    return 0;
}
static void reg_all(void) {
    reg_task("$t_setup", t_setup);
    reg_task("$t_check", t_check);
}
void (*vlog_startup_routines[])(void) = { reg_all, 0 };
"#;
    let sv = r#"
`timescale 1ns/1ns
module top;
  logic clk = 0;
  logic b = 0, c = 0, x = 1;
  always #5 clk = ~clk;
  A: assert property (@(posedge clk) b |-> ##2 c);
  B1: assert property (@(posedge clk) x);
  B2: assert property (@(posedge clk) x);
  initial begin
    $t_setup;
    b = 1;
    #10 b = 0; // t=10: A attempt 5 in flight
    $assertkill(0, top.A);
    #10 $assertoff(0, top.B1, top.B2); // t=20: B1 disable calls Disable(B2)
    #2 $asserton(0, top.B1, top.B2);   // t=22
    #2 $assertoff(0, top.B1, top.B2);  // t=24: B1 disable calls Enable(B2)
    #5; // t=29: posedge 25 starts B2
    $t_check;
    $display("RESULT: PASSED");
    $finish;
  end
endmodule
"#;
    let log = run_vpi_common("reentrant_ctl", c, sv);
    assert!(log.contains("ERRORS=0"), "C checks failed:\n{log}");
    assert!(log.contains("RESULT: PASSED"), "missing PASSED:\n{log}");
    let cbs: Vec<&str> = log.lines().filter(|l| l.starts_with("CB ")).collect();
    assert_eq!(
        cbs,
        [
            "CB B2 START t=5 start=5",
            "CB A KILL t=10 start=5",
            "CB A DISABLE t=10 start=10",
            "CB B2 START t=15 start=15",
            "CB B1 DISABLE t=20 start=20",
            "CB B2 DISABLE t=20 start=20",
            "CB B1 ENABLE t=22 start=22",
            "CB B2 ENABLE t=22 start=22",
            "CB B1 DISABLE t=24 start=24",
            "CB B2 ENABLE t=24 start=24",
            "CB B2 START t=25 start=25",
        ],
        "unexpected CB transcript:\n{log}"
    );
}

/// IEEE 1800-2017 Clause 39.5 / §20.12: `vpiAssertionSysReset` and
/// `vpiAssertionSysEnd` respect assertion lock, and `vpiAssertionSysReset`
/// notifies targets whose statement has not yet run.
#[test]
fn vpi_assertion_sys_reset_end_lock_and_unbound() {
    let c = r#"
static PLI_INT32 t_setup(PLI_BYTE8 *ud) {
    (void)ud;
    vpiHandle a = vpi_handle_by_name("top.A", NULL);
    vpiHandle b = vpi_handle_by_name("top.B", NULL);
    vpiHandle u = vpi_handle_by_name("top.U", NULL);
    CHECK(a && b && u, "handles");
    PLI_INT32 rs[] = {cbAssertionReset, cbAssertionKill, cbAssertionDisable,
                      cbAssertionStart, cbAssertionFailure};
    for (unsigned i = 0; i < sizeof rs / sizeof rs[0]; i++) {
        CHECK(vpi_register_assertion_cb(a, rs[i], on_cb, NULL) != NULL, "reg a");
        CHECK(vpi_register_assertion_cb(b, rs[i], on_cb, NULL) != NULL, "reg b");
    }
    CHECK(vpi_register_assertion_cb(u, cbAssertionReset, on_cb, NULL) != NULL, "reg u");
    return 0;
}
static PLI_INT32 t_sys_reset(PLI_BYTE8 *ud) {
    (void)ud;
    CHECK(vpi_control(vpiAssertionSysReset) == 1, "sys reset");
    return 0;
}
static PLI_INT32 t_sys_end(PLI_BYTE8 *ud) {
    (void)ud;
    CHECK(vpi_control(vpiAssertionSysEnd) == 1, "sys end");
    return 0;
}
static PLI_INT32 t_check(PLI_BYTE8 *ud) {
    (void)ud;
    vpi_printf("ERRORS=%d\n", errors);
    return 0;
}
static void reg_all(void) {
    reg_task("$t_setup", t_setup);
    reg_task("$t_sys_reset", t_sys_reset);
    reg_task("$t_sys_end", t_sys_end);
    reg_task("$t_check", t_check);
}
void (*vlog_startup_routines[])(void) = { reg_all, 0 };
"#;
    let sv = r#"
`timescale 1ns/1ns
module top;
  logic clk = 0;
  logic b = 0, c = 0;
  always #5 clk = ~clk;
  A: assert property (@(posedge clk) b |-> ##2 c) else $display("A_ACTION_FAIL t=%0t", $time);
  B: assert property (@(posedge clk) b |-> ##2 c) else $display("B_ACTION_FAIL t=%0t", $time);
  // Unclocked concurrent assertion has a VPI handle but no sva_sites entry.
  U: assert property (1'b1);
  initial begin
    $t_setup;
    $t_sys_reset; // t=0: A, B, and unbound U all receive RESET
    #2 b = 1;     // t=2: attempts start at t=5
    #5 b = 0;     // t=7
    $assertcontrol(1, 1, 7, 0, top.A); // Lock A
    #3 $t_sys_reset; // t=10: A locked -> no RESET, attempt 5 stays in flight
    #20;             // t=30: A attempt 5 fails at t=25
    $assertcontrol(2, 1, 7, 0, top.A); // Unlock A
    #2 $t_sys_reset; // t=32: A now gets RESET
    #2 b = 1;        // t=34: attempts start at t=35
    #3 b = 0;        // t=37
    $assertcontrol(1, 1, 7, 0, top.A); // Lock A again
    #3 $t_sys_end;   // t=40: B killed & disabled; locked A keeps attempt 35
    #20;             // t=60: A attempt 35 fails at t=55
    $t_check;
    $display("RESULT: PASSED");
    $finish;
  end
endmodule
"#;
    let log = run_vpi_common("sys_lock", c, sv);
    assert!(log.contains("ERRORS=0"), "C checks failed:\n{log}");
    assert!(log.contains("RESULT: PASSED"), "missing PASSED:\n{log}");
    let cbs: Vec<&str> = log.lines().filter(|l| l.starts_with("CB ")).collect();
    assert_eq!(
        cbs,
        [
            "CB A RESET t=0 start=0",
            "CB B RESET t=0 start=0",
            "CB U RESET t=0 start=0",
            "CB A START t=5 start=5",
            "CB B START t=5 start=5",
            "CB B RESET t=10 start=10",
            "CB U RESET t=10 start=10",
            "CB A START t=15 start=15",
            "CB B START t=15 start=15",
            "CB A START t=25 start=25",
            "CB A FAILURE t=25 start=5",
            "CB B START t=25 start=25",
            "CB A RESET t=32 start=32",
            "CB B RESET t=32 start=32",
            "CB U RESET t=32 start=32",
            "CB A START t=35 start=35",
            "CB B START t=35 start=35",
            "CB B KILL t=40 start=35",
            "CB B DISABLE t=40 start=40",
        ],
        "unexpected CB transcript:\n{log}"
    );
    assert_eq!(log.matches("A_ACTION_FAIL t=25").count(), 1, "{log}");
    assert_eq!(log.matches("A_ACTION_FAIL t=55").count(), 1, "{log}");
    assert!(!log.contains("B_ACTION_FAIL"), "{log}");
}

/// IEEE 1800-2017 Clause 39.5: `vpiAssertionEnableStep` / `DisableStep`
/// called before `vpi_register_assertion_cb` takes effect when callbacks are
/// later registered, and `vpiAssertionSysReset` clears pending step state.
#[test]
fn vpi_assertion_enable_step_before_reg() {
    let c = r#"
static PLI_INT32 t_setup(PLI_BYTE8 *ud) {
    (void)ud;
    vpiHandle a1 = vpi_handle_by_name("top.A1", NULL);
    vpiHandle a2 = vpi_handle_by_name("top.A2", NULL);
    vpiHandle a3 = vpi_handle_by_name("top.A3", NULL);
    CHECK(a1 && a2 && a3, "handles");
    s_vpi_time t = {vpiSimTime, 0, 5, 0.0};
    CHECK(vpi_control(vpiAssertionEnableStep, a3, &t, vpiAssertionClockSteps) == 1, "en step a3");
    CHECK(vpi_control(vpiAssertionSysReset) == 1, "sys reset clears step");
    CHECK(vpi_control(vpiAssertionEnableStep, a1, &t, vpiAssertionClockSteps) == 1, "en step a1");
    CHECK(vpi_control(vpiAssertionEnableStep, a2, &t, vpiAssertionClockSteps) == 1, "en step a2");
    CHECK(vpi_control(vpiAssertionDisableStep, a2, &t) == 1, "dis step a2");
    PLI_INT32 rs[] = {cbAssertionStart, cbAssertionStepSuccess, cbAssertionSuccess};
    for (unsigned i = 0; i < sizeof rs / sizeof rs[0]; i++) {
        CHECK(vpi_register_assertion_cb(a1, rs[i], on_cb, NULL) != NULL, "reg a1");
        CHECK(vpi_register_assertion_cb(a2, rs[i], on_cb, NULL) != NULL, "reg a2");
        CHECK(vpi_register_assertion_cb(a3, rs[i], on_cb, NULL) != NULL, "reg a3");
    }
    return 0;
}
static PLI_INT32 t_check(PLI_BYTE8 *ud) {
    (void)ud;
    vpi_printf("ERRORS=%d\n", errors);
    return 0;
}
static void reg_all(void) {
    reg_task("$t_setup", t_setup);
    reg_task("$t_check", t_check);
}
void (*vlog_startup_routines[])(void) = { reg_all, 0 };
"#;
    let sv = r#"
`timescale 1ns/1ns
module top;
  logic clk = 0;
  logic b = 0, c = 0;
  always #5 clk = ~clk;
  A1: assert property (@(posedge clk) b |-> ##2 c);
  A2: assert property (@(posedge clk) b |-> ##2 c);
  A3: assert property (@(posedge clk) b |-> ##2 c);
  initial begin
    $t_setup;
    b = 1;
    #10 b = 0; // t=10
    #10 c = 1; // t=20
    #10;       // t=30
    $t_check;
    $display("RESULT: PASSED");
    $finish;
  end
endmodule
"#;
    let log = run_vpi_common("step_before_reg", c, sv);
    assert!(log.contains("ERRORS=0"), "C checks failed:\n{log}");
    assert!(log.contains("RESULT: PASSED"), "missing PASSED:\n{log}");
    let cbs: Vec<&str> = log.lines().filter(|l| l.starts_with("CB ")).collect();
    assert_eq!(
        cbs,
        [
            "CB A1 START t=5 start=5",
            "CB A1 STEP_OK t=5 start=5",
            "CB A2 START t=5 start=5",
            "CB A3 START t=5 start=5",
            "CB A1 START t=15 start=15",
            "CB A1 STEP_OK t=15 start=5",
            "CB A2 START t=15 start=15",
            "CB A3 START t=15 start=15",
            "CB A1 START t=25 start=25",
            "CB A1 SUCCESS t=25 start=5",
            "CB A2 START t=25 start=25",
            "CB A2 SUCCESS t=25 start=5",
            "CB A3 START t=25 start=25",
            "CB A3 SUCCESS t=25 start=5",
        ],
        "unexpected CB transcript:\n{log}"
    );
}

/// IEEE 1800-2017 Clause 39.4.2 / 39.5: unsupported lock/action callback
/// reasons are refused with `vpi_chk_error`, and `vpi_control` on a
/// `vpiPropertyInst` handle is refused while leaving the directive unaffected.
#[test]
fn vpi_assertion_refused_reasons_and_prop_inst_control() {
    let c = r#"
static PLI_INT32 t_setup(PLI_BYTE8 *ud) {
    (void)ud;
    vpiHandle top = vpi_handle_by_name("top", NULL);
    vpiHandle a = vpi_handle_by_name("top.A", NULL);
    vpiHandle it = vpi_iterate(vpiPropertyInst, top);
    vpiHandle inst = it ? vpi_scan(it) : NULL;
    if (it) vpi_free_object(it);
    CHECK(a && inst, "handles");
    PLI_INT32 refused[] = {
        cbAssertionEnablePassAction, cbAssertionEnableFailAction,
        cbAssertionDisablePassAction, cbAssertionDisableFailAction,
        cbAssertionEnableNonvacuousAction, cbAssertionDisableVacuousAction,
        cbAssertionLock, cbAssertionUnlock
    };
    s_vpi_error_info err;
    for (unsigned i = 0; i < sizeof refused / sizeof refused[0]; i++) {
        CHECK(vpi_register_assertion_cb(a, refused[i], on_cb, NULL) == NULL, "refused reason");
        CHECK(vpi_chk_error(&err) != 0, "error recorded for reason");
    }
    s_vpi_time t = {vpiSimTime, 0, 0, 0.0};
    CHECK(vpi_control(vpiAssertionDisable, inst) == 0, "disable inst refused");
    CHECK(vpi_chk_error(&err) != 0, "error recorded for disable inst");
    CHECK(vpi_control(vpiAssertionEnable, inst) == 0, "enable inst refused");
    CHECK(vpi_chk_error(&err) != 0, "error recorded for enable inst");
    CHECK(vpi_control(vpiAssertionReset, inst) == 0, "reset inst refused");
    CHECK(vpi_chk_error(&err) != 0, "error recorded for reset inst");
    CHECK(vpi_control(vpiAssertionKill, inst, &t) == 0, "kill inst refused");
    CHECK(vpi_chk_error(&err) != 0, "error recorded for kill inst");
    CHECK(vpi_control(vpiAssertionEnableStep, inst, &t, vpiAssertionClockSteps) == 0, "step inst refused");
    CHECK(vpi_chk_error(&err) != 0, "error recorded for step inst");
    CHECK(vpi_register_assertion_cb(a, cbAssertionStart, on_cb, NULL) != NULL, "reg start");
    CHECK(vpi_register_assertion_cb(a, cbAssertionSuccess, on_cb, NULL) != NULL, "reg success");
    return 0;
}
static PLI_INT32 t_check(PLI_BYTE8 *ud) {
    (void)ud;
    vpi_printf("ERRORS=%d\n", errors);
    return 0;
}
static void reg_all(void) {
    reg_task("$t_setup", t_setup);
    reg_task("$t_check", t_check);
}
void (*vlog_startup_routines[])(void) = { reg_all, 0 };
"#;
    let sv = r#"
`timescale 1ns/1ns
module top;
  logic clk = 0;
  logic x = 1;
  always #5 clk = ~clk;
  property p;
    @(posedge clk) x;
  endproperty
  A: assert property (p);
  initial begin
    $t_setup;
    #10;
    $t_check;
    $display("RESULT: PASSED");
    $finish;
  end
endmodule
"#;
    let log = run_vpi_common("refused_reasons", c, sv);
    assert!(log.contains("ERRORS=0"), "C checks failed:\n{log}");
    assert!(log.contains("RESULT: PASSED"), "missing PASSED:\n{log}");
    let cbs: Vec<&str> = log.lines().filter(|l| l.starts_with("CB ")).collect();
    assert_eq!(
        cbs,
        ["CB A START t=5 start=5", "CB A SUCCESS t=5 start=5"],
        "unexpected CB transcript:\n{log}"
    );
}

/// IEEE 1800-2017 Clause 39.5: `vpiAssertionKill` with `vpiScaledRealTime`
/// scales the attempt start time by the assertion's containing instance time
/// unit (`1ns` = 1000 ticks under `` `timescale 1ns/1ps ``).
#[test]
fn vpi_assertion_scaled_real_kill() {
    let c = r#"
static vpiHandle a_h = NULL;
static PLI_INT32 t_setup(PLI_BYTE8 *ud) {
    (void)ud;
    a_h = vpi_handle_by_name("top.A", NULL);
    CHECK(a_h != NULL, "A handle");
    PLI_INT32 rs[] = {cbAssertionStart, cbAssertionFailure, cbAssertionKill};
    for (unsigned i = 0; i < sizeof rs / sizeof rs[0]; i++)
        CHECK(vpi_register_assertion_cb(a_h, rs[i], on_cb, NULL) != NULL, "reg");
    return 0;
}
static PLI_INT32 t_kill_real(PLI_BYTE8 *ud) {
    (void)ud;
    s_vpi_time t = {vpiScaledRealTime, 0, 0, 5.0};
    CHECK(vpi_control(vpiAssertionKill, a_h, &t) == 1, "scaled real kill");
    return 0;
}
static PLI_INT32 t_check(PLI_BYTE8 *ud) {
    (void)ud;
    vpi_printf("ERRORS=%d\n", errors);
    return 0;
}
static void reg_all(void) {
    reg_task("$t_setup", t_setup);
    reg_task("$t_kill_real", t_kill_real);
    reg_task("$t_check", t_check);
}
void (*vlog_startup_routines[])(void) = { reg_all, 0 };
"#;
    let sv = r#"
`timescale 1ns/1ps
module top;
  logic clk = 0;
  logic b = 1, c = 0;
  always #5 clk = ~clk; // posedge at 5ns = 5000 ticks
  A: assert property (@(posedge clk) b |-> ##2 c);
  initial begin
    $t_setup;
    #6 $t_kill_real; // t=6ns (6000 ticks): kill attempt started at 5.0ns (5000 ticks)
    #4 b = 0;        // t=10ns
    #20;             // t=30ns (past 25ns failure point if not killed)
    $t_check;
    $display("RESULT: PASSED");
    $finish;
  end
endmodule
"#;
    let log = run_vpi_common("scaled_real_kill", c, sv);
    assert!(log.contains("ERRORS=0"), "C checks failed:\n{log}");
    assert!(log.contains("RESULT: PASSED"), "missing PASSED:\n{log}");
    let cbs: Vec<&str> = log.lines().filter(|l| l.starts_with("CB ")).collect();
    assert_eq!(
        cbs,
        [
            "CB A START t=5000 start=5000",
            "CB A KILL t=6000 start=5000",
            "CB A START t=15000 start=15000",
            "CB A START t=25000 start=25000",
        ],
        "unexpected CB transcript:\n{log}"
    );
}
