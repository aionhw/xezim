//! IEEE 1800-2023 §16.5.1: an assertion attempt at time 0 reads default
//! sampled values, also under a clock a VPI client raises at time 0, as
//! cocotb's Python `Clock` does from cbReadWriteSynch. The sampled value of a
//! variable in time slot 0 is its declaration initializer, else its type's
//! default. A continuous assignment, an `always_comb`, an `initial` block and
//! a VPI write in the slot all come after its Preponed region, so their
//! values are not the time-0 samples. §16.9.3: `$past` at the first tick
//! returns default sampled values, of an expression's operands too, and at
//! the next tick the time-0 samples. xezim took the time-0 samples after the
//! time-0 settle, so the operands of continuous assignments and
//! `always_comb` read their driven values and those attempts passed.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Raises `top.clk` and sets `top.a_in` to 1 from a cbReadWriteSynch at time
/// 0, the way cocotb applies its first writes and its Python `Clock`, then
/// toggles the clock every 5 ns.
const DRIVER_C: &str = r#"
#include <string.h>
#include "vpi_user.h"

static vpiHandle clk, in;
static int level = 1;

static void put(vpiHandle h, int v) {
    s_vpi_value val;
    s_vpi_time t = {vpiSimTime, 0, 0, 0.0};
    val.format = vpiIntVal;
    val.value.integer = v;
    vpi_put_value(h, &val, &t, vpiNoDelay);
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

static PLI_INT32 timer(p_cb_data cb) {
    (void)cb;
    level = !level;
    put(clk, level);
    at(timer, cbAfterDelay, 5);
    return 0;
}

static PLI_INT32 rw(p_cb_data cb) {
    (void)cb;
    put(in, 1);
    put(clk, 1);
    return 0;
}

static PLI_INT32 start(p_cb_data cb) {
    (void)cb;
    clk = vpi_handle_by_name("top.clk", NULL);
    in = vpi_handle_by_name("top.a_in", NULL);
    at(rw, cbReadWriteSynch, 0);
    at(timer, cbAfterDelay, 5);
    return 0;
}

static void boot(void) { at(start, cbStartOfSimulation, 0); }

void (*vlog_startup_routines[])(void) = {boot, 0};
"#;

/// One assertion per kind of operand; each passes when the operand's sampled
/// value is the value the slot writes. At 10 ns, `$past` reads the time-0
/// samples.
const TOP_SV: &str = r#"
module top;
  logic clk, a_in;
  logic r;                         // never written: x
  logic ri = 1'b1;                 // declaration initializer
  int   n;                         // 2-state default: 0
  wire  w = 1'b1;                  // net declaration assignment
  logic c; assign c = ri;          // continuous assignment
  logic g; always_comb g = ri;     // always_comb
  logic f; initial f = 1'b1;       // initial block
  logic m [2]; always_comb m[0] = ri;  // an array cell
  // A procedural $past at the first tick: the default of `a_in & pb`, from
  // both operands' defaults.
  logic pb; always_comb pb = ri;
  always @(posedge clk) if ($time == 0) $display("T|ppast0|%b", $past(a_in & pb));
  a_r:  assert property (@(posedge clk) r === 1'bx) $display("T|r|pass|%0t", $time); else $display("T|r|fail|%0t", $time);
  a_ri: assert property (@(posedge clk) ri === 1'b1) $display("T|ri|pass|%0t", $time); else $display("T|ri|fail|%0t", $time);
  a_n:  assert property (@(posedge clk) n === 0) $display("T|n|pass|%0t", $time); else $display("T|n|fail|%0t", $time);
  a_w:  assert property (@(posedge clk) w === 1'b1) $display("T|w|pass|%0t", $time); else $display("T|w|fail|%0t", $time);
  a_c:  assert property (@(posedge clk) c === 1'b1) $display("T|c|pass|%0t", $time); else $display("T|c|fail|%0t", $time);
  a_g:  assert property (@(posedge clk) g === 1'b1) $display("T|g|pass|%0t", $time); else $display("T|g|fail|%0t", $time);
  a_f:  assert property (@(posedge clk) f === 1'b1) $display("T|f|pass|%0t", $time); else $display("T|f|fail|%0t", $time);
  a_m:  assert property (@(posedge clk) m[0] === 1'b1) $display("T|m|pass|%0t", $time); else $display("T|m|fail|%0t", $time);
  a_in_a: assert property (@(posedge clk) a_in === 1'b1) $display("T|in|pass|%0t", $time); else $display("T|in|fail|%0t", $time);
  a_p:  assert property (@(posedge clk) $time != 10 || 0)
          else $display("T|past|%0t|%b%b%b", $time, $past(a_in), $past(c), $past(ri));
  a_p0: assert property (@(posedge clk) $time != 0 || 0)
          else $display("T|past0|%b%b", $past(a_in), $past(a_in | 1'b0));
  initial #12 $finish;
endmodule
"#;

/// A scratch directory holding the compiled driver and `top.sv`.
fn setup() -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("xezim_vpi_sva_t0_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let c = dir.join("drv.c");
    let so = dir.join("drv.so");
    std::fs::write(&c, DRIVER_C).unwrap();
    std::fs::write(dir.join("top.sv"), TOP_SV).unwrap();
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
    (dir, so)
}

#[test]
fn time0_attempt_reads_default_sampled_values() {
    let (dir, so) = setup();
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .arg("--vpi-lib")
        .arg(&so)
        .args(["--no-cache", "-s", "top", "top.sv"])
        .current_dir(&dir)
        .output()
        .expect("failed to run xezim");
    let _ = std::fs::remove_dir_all(&dir);
    let mut got: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.starts_with("T|"))
        .map(str::to_string)
        .collect();
    got.sort();
    let mut want = vec![
        "T|c|fail|0",
        "T|c|pass|10",
        "T|f|fail|0",
        "T|f|pass|10",
        "T|g|fail|0",
        "T|g|pass|10",
        "T|in|fail|0",
        "T|in|pass|10",
        "T|m|fail|0",
        "T|m|pass|10",
        "T|n|pass|0",
        "T|n|pass|10",
        "T|past0|xx",
        "T|past|10|xx1",
        "T|ppast0|x",
        "T|r|pass|0",
        "T|r|pass|10",
        "T|ri|pass|0",
        "T|ri|pass|10",
        "T|w|fail|0",
        "T|w|pass|10",
    ];
    want.sort();
    assert_eq!(got, want);
}
