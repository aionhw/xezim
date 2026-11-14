//! IEEE 1800 §14.4, §14.13 and §38.36: callback-driven clocks retain
//! slot-entry #1step samples and deliver clocking events after the same edge's NBA.
use std::path::Path;
use std::process::Command;

fn run(mode: &str, same: bool) {
    let tag = format!("{mode}_{}", u8::from(same));
    let dir =
        std::env::temp_dir().join(format!("xezim_clocking357_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create VPI scratch");
    std::fs::write(dir.join("clock.c"), CALLBACKS).unwrap();
    std::fs::write(dir.join("top.sv"), DESIGN).unwrap();
    let so = dir.join("clock.so");
    let compiled = Command::new(std::env::var_os("CC").unwrap_or_else(|| "clang".into()))
        .args(["-shared", "-fPIC", "-Wall", "-Wextra", "-I"])
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("include"))
        .arg(dir.join("clock.c"))
        .arg("-o")
        .arg(&so)
        .output()
        .expect("compile clocking VPI module");
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let mut command = Command::new(env!("CARGO_BIN_EXE_xezim"));
    command
        .args(["--no-cache", "-s", "top", "--vpi-lib"])
        .arg(&so)
        .arg("top.sv")
        .current_dir(&dir)
        .env("CLK", mode)
        .env("SAME", if same { "1" } else { "0" });
    if same {
        command.arg("+same");
    }
    let output = command.output().expect("run callback clocking regression");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{tag}: {stdout}\n{stderr}");
    assert!(
        !stdout.lines().any(|line| line.starts_with("FAIL")),
        "{tag}: {stdout}\n{stderr}"
    );
    let lines: Vec<_> = stdout
        .lines()
        .filter(|line| line.starts_with("T|"))
        .collect();
    assert_eq!(
        lines,
        [
            "T|initial 20 0",
            "T|always 20 0",
            "T|body 20 0",
            "T|zero 20 0",
            "T|negedge 20 0"
        ],
        "{tag}: {stdout}\n{stderr}"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn callback_clocks_deliver_sampled_events_in_their_own_slot() {
    for (mode, same) in [
        ("rw", false),
        ("delay", false),
        ("delay", true),
        ("rw", true),
    ] {
        run(mode, same);
    }
}

const CALLBACKS: &str = r#"#include <stdlib.h>
#include <string.h>
#include "vpi_user.h"

static vpiHandle clk, d;
static int level, n, clock_reason, same;
static const int pat[] = {0, 1, 0, 1, 1, 0, 0, 1, 0, 1, 1, 1, 0, 0, 1, 0, 1, 0, 0, 1};

static void put(vpiHandle h, int v) {
    s_vpi_value val;
    val.format = vpiIntVal;
    val.value.integer = v;
    vpi_put_value(h, &val, NULL, vpiNoDelay);
}

static void at(PLI_INT32 (*rtn)(p_cb_data), int reason, unsigned delay) {
    s_cb_data cb;
    s_vpi_time t = {vpiSimTime, 0, delay, 0.0};
    memset(&cb, 0, sizeof cb);
    cb.reason = reason;
    cb.cb_rtn = rtn;
    cb.time = &t;
    vpi_register_cb(&cb);
}

static PLI_INT32 rw_d(p_cb_data cb) { (void)cb; put(d, pat[(n / 2) % 20]); return 0; }
static PLI_INT32 rw_c(p_cb_data cb) { (void)cb; put(clk, level); return 0; }

static PLI_INT32 timer(p_cb_data cb) {
    (void)cb;
    level ^= 1;
    if (level && same)
        put(d, pat[(n / 2) % 20]);
    if (clock_reason)
        at(rw_c, clock_reason, 0);
    else
        put(clk, level);
    if (!level && !same)
        at(rw_d, cbReadWriteSynch, 0);
    if (++n < 40)
        at(timer, cbAfterDelay, 5);
    return 0;
}

static PLI_INT32 start(p_cb_data cb) {
    const char *c = getenv("CLK");
    (void)cb;
    clock_reason = c && !strcmp(c, "delay") ? 0 : cbReadWriteSynch;
    same = getenv("SAME") && !strcmp(getenv("SAME"), "1");
    clk = vpi_handle_by_name("top.clk", NULL);
    d = vpi_handle_by_name("top.d", NULL);
    at(timer, cbAfterDelay, 5);
    return 0;
}

static void boot(void) { at(start, cbStartOfSimulation, 0); }
void (*vlog_startup_routines[])(void) = {boot, 0};
"#;

const DESIGN: &str = r#"`timescale 1ns/1ns
module top;
  logic clk = 0, d = 0, q = 0, q2 = 0;
  int n = 0, fails = 0;
  time tpos = 0;
  bit same;
  initial same = $test$plusargs("same");
  always_ff @(posedge clk) begin q <= d; q2 <= q; end
  always @(posedge clk) tpos = $time;
  clocking cb @(posedge clk);
    input q, d;
  endclocking
  // A process waiting on cb resumes in the time slot of the edge, once the
  // block has sampled (§14.13). The #1step inputs are sampled in the Preponed
  // region (§14.4): cb.q is q from before the edge, the value q2 has just
  // taken; cb.d is d from before the edge, the value q has just taken, or
  // with +same (d written in the edge's own cbAfterDelay) the old d, q2.
  initial forever begin
    @(cb);
    n++;
    if ($time != tpos || cb.q !== q2 || cb.d !== (same ? q2 : q)) begin
      fails++;
      $display("FAIL t=%0t edge=%0t cb.q=%b q2=%b cb.d=%b q=%b", $time, tpos, cb.q, q2, cb.d, q);
    end
  end

  clocking cb0 @(posedge clk); input #0 q,d; endclocking
  clocking cbn @(negedge clk); input q,d; endclocking
  int na=0,nb=0,nz=0,nn=0,fa=0,fb=0,fz=0,fn=0;
  always @(cb) begin
    na++;
    if ($time!=tpos || cb.q!==q2 || cb.d!==(same?q2:q)) fa++;
  end
  always begin
    @(cb); nb++;
    if ($time!=tpos || cb.q!==q2 || cb.d!==(same?q2:q)) fb++;
  end
  initial forever begin
    @(cb0); nz++;
    if ($time!=tpos || cb0.q!==q || cb0.d!==d) fz++;
  end
  initial forever begin
    @(cbn); nn++;
    if ($time%10!=0 || cbn.q!==q || cbn.d!==q) fn++;
  end
  initial #203 begin
    $display("T|initial %0d %0d",n,fails);
    $display("T|always %0d %0d",na,fa);
    $display("T|body %0d %0d",nb,fb);
    $display("T|zero %0d %0d",nz,fz);
    $display("T|negedge %0d %0d",nn,fn);
    $finish;
  end
endmodule
"#;
