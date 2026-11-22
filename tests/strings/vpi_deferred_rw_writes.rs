//! IEEE 1800-2023 §4.5, §16.4.1, §16.4.2: a deferred assertion's report
//! matures in the Observed region (`#0`) or the Postponed region (`final`),
//! which a time slot reaches only once every region through Post-NBA is
//! empty. A cbReadWriteSynch still due in the slot runs first, and a block
//! its writes re-run flushes the report it queued on the old values.
//!
//! cocotb applies its queued writes from a cbReadWriteSynch that it registers
//! while another one runs, so at time 0 that callback is still due when the
//! slot's other regions are empty. The time-0 settle runs each
//! `always_comb` on x inputs and queues a failure; the driver's writes then
//! re-run it, and only the passing report may mature.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Writes `top.a` = 1 and `top.b` = 2 at time 0. `T0_WRITE` picks the
/// callback that writes: `rw`, a cbReadWriteSynch registered from
/// cbStartOfSimulation, or `rw2`, one registered from inside another
/// cbReadWriteSynch, the way cocotb applies its writes. `rw3` adds a third
/// level. `rw2clk` is `rw2` raising `top.clk` too.
const DRIVER_C: &str = r#"
#include <stdlib.h>
#include <string.h>
#include "vpi_user.h"

static void put(const char *n, int v) {
    s_vpi_value val;
    s_vpi_time t = {vpiSimTime, 0, 0, 0.0};
    val.format = vpiIntVal;
    val.value.integer = v;
    vpi_put_value(vpi_handle_by_name((PLI_BYTE8 *)n, NULL), &val, &t, vpiNoDelay);
}

static void at(PLI_INT32 (*rtn)(p_cb_data), int reason) {
    s_cb_data cb;
    s_vpi_time t = {vpiSimTime, 0, 0, 0.0};
    memset(&cb, 0, sizeof cb);
    cb.reason = reason;
    cb.cb_rtn = rtn;
    cb.time = &t;
    vpi_register_cb(&cb);
}

static PLI_INT32 rw(p_cb_data cb) {
    (void)cb;
    const char *m = getenv("T0_WRITE");
    put("top.a", 1);
    put("top.b", 2);
    if (m && !strcmp(m, "rw2clk"))
        put("top.clk", 1);
    return 0;
}

static PLI_INT32 outer(p_cb_data cb) {
    (void)cb;
    at(rw, cbReadWriteSynch);
    return 0;
}

static PLI_INT32 outer2(p_cb_data cb) {
    (void)cb;
    at(outer, cbReadWriteSynch);
    return 0;
}

static PLI_INT32 start(p_cb_data cb) {
    (void)cb;
    const char *m = getenv("T0_WRITE");
    at(m && !strcmp(m, "rw3") ? outer2 : m && strncmp(m, "rw2", 3) == 0 ? outer : rw,
       cbReadWriteSynch);
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

/// The program's waiter runs in the Reactive region, after the Observed
/// region where the `#0` report matures, and ends the run there.
const TOP_SV: &str = r#"
program fin (input logic clk);
  initial begin
    @(posedge clk);
    $finish;
  end
endprogram

module top;
  logic clk = 0;
  fin f (.clk);
  logic [3:0] a, b;
  logic [4:0] s;
  assign s = a + b;
  always_comb assert #0 (s == a + b) $display("T|zero|pass|%0t", $time);
    else $display("T|zero|fail|%0t", $time);
  always_comb assert final (s == a + b) $display("T|final|pass|%0t", $time);
    else $display("T|final|fail|%0t", $time);
  initial #5 $finish;
endmodule
"#;

/// A scratch directory holding the compiled driver and `top.sv`.
fn setup() -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("xezim_vpi_deferred_rw_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let c = dir.join("drv.c");
    let so = dir.join("drv.so");
    std::fs::write(&c, DRIVER_C).unwrap();
    std::fs::write(dir.join("top.sv"), TOP_SV).unwrap();
    let include = Path::new(env!("CARGO_MANIFEST_DIR")).join("include");
    let ok = Command::new("cc")
        .args(["-shared", "-fPIC", "-Wall", "-I"])
        .arg(&include)
        .arg(&c)
        .arg("-o")
        .arg(&so)
        .status()
        .expect("failed to launch cc")
        .success();
    assert!(ok, "cc failed for {}", c.display());
    (dir, so)
}

/// The `T|` lines of one run, in order.
fn run(dir: &Path, so: &Path, mode: &str) -> Vec<String> {
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .arg("--vpi-lib")
        .arg(so)
        .args(["--no-cache", "-s", "top", "top.sv"])
        .env("T0_WRITE", mode)
        .current_dir(dir)
        .output()
        .expect("failed to run xezim");
    assert!(
        out.status.success(),
        "T0_WRITE={mode}: xezim failed\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.starts_with("T|"))
        .map(str::to_string)
        .collect()
}

#[test]
fn time0_read_write_synch_writes_flush_deferred_reports() {
    let (dir, so) = setup();
    // With `rw2clk` the program ends the run at time 0, after the `#0`
    // report matured and before the Postponed region of the `final` one.
    let want = |m: &str| -> Vec<String> {
        let all = ["T|zero|pass|0", "T|final|pass|0"];
        let n = if m == "rw2clk" { 1 } else { 2 };
        all[..n].iter().map(|s| s.to_string()).collect()
    };
    let bad: Vec<String> = ["rw", "rw2", "rw3", "rw2clk"]
        .iter()
        .map(|m| (m, run(&dir, &so, m)))
        .filter(|(m, got)| *got != want(m))
        .map(|(m, got)| format!("T0_WRITE={m}: got {got:?}, want {:?}", want(m)))
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}
