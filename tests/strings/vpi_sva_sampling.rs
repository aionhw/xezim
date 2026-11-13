//! IEEE 1800-2017 §16.5.1 sampled values under a clock driven through VPI
//! (§38.34 `vpi_put_value`), the way cocotb drives one.
//!
//! A concurrent assertion samples its variables in the Preponed region of the
//! slot its clock ticks in, before that slot's Pre-Active VPI callbacks
//! (§4.4.3.2) and before its NBA updates, and is evaluated in the Observed
//! region of the delta that made the edge (§4.4.2). cocotb's Python `Clock`
//! writes the clock from cbReadWriteSynch, after the NBA region; its GPI
//! clock writes it from cbAfterDelay. Under the first, the assertions ticked
//! in the next delta or the next slot and read every flop's new value; a
//! value written in the edge's own cbAfterDelay counted as already sampled.
//! The properties hold on the sampled values in every run, so none may fail.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Drives `top.clk`, `top.d` and `top.rst`. `SVA_CLK` picks the routine that
/// writes the clock: `delay` (the cbAfterDelay timer), `rw` (the timer queues
/// it for cbReadWriteSynch), `vc` (a cbValueChange on `top.tick`) or `none`
/// (the design drives it). `SVA_DATA` picks where `d` changes: `rw`
/// (cbReadWriteSynch after the falling edge), `delay` (the falling edge's
/// timer), `vc` (a cbValueChange on the clock's fall) or `same` (the rising
/// edge's own timer, before the clock). `SVA_FLAG` is the `vpi_put_value`
/// flag: `nodelay`, `inertial` or `force`. With `SVA_FINISH=<t>`, a
/// cbValueChange on the clock calls `vpi_control(vpiFinish)` at its first rise
/// at or after `t`, as cocotb does when a test ends on `RisingEdge`. With
/// `SVA_RST=<k>`, `rst` is released at the clock's k-th rise instead, from a
/// cbReadWriteSynch its cbValueChange queues: a cocotb write after
/// `RisingEdge`. With `SVA_GLITCH=1`, a cbReadWriteSynch queued the same way
/// lowers the clock again after each rise, in the same time slot. With
/// `SVA_RWNOP=1`, one that does nothing is queued at each rise, as cocotb
/// does whenever a test has a write pending or awaits `ReadWrite`. With
/// `SVA_CHAIN=<n>`, the third rise (`SVA_CHAIN_AT`) queues a
/// cbReadWriteSynch (`SVA_CHAIN_CB=nba`: a cbNBASynch) that queues another,
/// `n` times over, all in that time slot.
const DRIVER_C: &str = r#"
#include <stdlib.h>
#include <string.h>
#include "vpi_user.h"

static vpiHandle clk, d, rst;
static int flag = vpiNoDelay;
static const char *cmode = "delay", *dmode = "rw";
static const int pat[] = {0, 1, 0, 1, 1, 0, 0, 1, 0, 1, 1, 1, 0, 0, 1, 0, 1, 0, 0, 1};
#define NPAT (int)(sizeof pat / sizeof pat[0])
static int level, cyc, steps, rst_edge, rises;

static const char *env(const char *n, const char *dflt) {
    const char *v = getenv(n);
    return v && *v ? v : dflt;
}

static void put(vpiHandle h, int v) {
    s_vpi_value val;
    s_vpi_time t = {vpiSimTime, 0, 0, 0.0};
    val.format = vpiIntVal;
    val.value.integer = v;
    vpi_put_value(h, &val, &t, flag);
}

static void at(PLI_INT32 (*rtn)(p_cb_data), int reason, unsigned delay, vpiHandle obj) {
    s_cb_data cb;
    s_vpi_time t = {vpiSimTime, 0, delay, 0.0};
    s_vpi_value v;
    memset(&cb, 0, sizeof cb);
    v.format = vpiIntVal;
    cb.reason = reason;
    cb.cb_rtn = rtn;
    cb.obj = obj;
    cb.time = &t;
    cb.value = &v;
    vpi_register_cb(&cb);
}

static void data(void) {
    put(d, pat[cyc % NPAT]);
    if (cyc == 3 && !rst_edge)
        put(rst, 0);
    cyc++;
}

static PLI_INT32 rw_data(p_cb_data cb) { (void)cb; data(); return 0; }
static PLI_INT32 rw_clk(p_cb_data cb) { (void)cb; put(clk, level); return 0; }

static void step(int lvl) {
    level = lvl;
    if (lvl && !strcmp(dmode, "same"))
        data();
    if (!strcmp(cmode, "rw"))
        at(rw_clk, cbReadWriteSynch, 0, NULL);
    else
        put(clk, lvl);
    if (!lvl && !strcmp(dmode, "rw"))
        at(rw_data, cbReadWriteSynch, 0, NULL);
    if (!lvl && !strcmp(dmode, "delay"))
        data();
}

static PLI_INT32 timer(p_cb_data cb) {
    (void)cb;
    step(!level);
    if (++steps < 2 * NPAT)
        at(timer, cbAfterDelay, 5, NULL);
    return 0;
}

static PLI_INT32 tick_vc(p_cb_data cb) {
    if (steps++ < 2 * NPAT)
        step(cb->value->value.integer & 1);
    return 0;
}

static PLI_INT32 fin_vc(p_cb_data cb) {
    s_vpi_time t = {vpiSimTime, 0, 0, 0.0};
    vpi_get_time(NULL, &t);
    if ((cb->value->value.integer & 1) && (int)t.low >= atoi(env("SVA_FINISH", "0")))
        vpi_control(vpiFinish, 0);
    return 0;
}

static PLI_INT32 clk_vc(p_cb_data cb) {
    if ((cb->value->value.integer & 1) == 0)
        data();
    return 0;
}

static PLI_INT32 rw_rst(p_cb_data cb) { (void)cb; put(rst, 0); return 0; }

static PLI_INT32 rst_vc(p_cb_data cb) {
    if ((cb->value->value.integer & 1) && ++rises == rst_edge)
        at(rw_rst, cbReadWriteSynch, 0, NULL);
    return 0;
}

static PLI_INT32 rw_low(p_cb_data cb) { (void)cb; put(clk, 0); return 0; }

static PLI_INT32 glitch_vc(p_cb_data cb) {
    if (cb->value->value.integer & 1)
        at(rw_low, cbReadWriteSynch, 0, NULL);
    return 0;
}

static PLI_INT32 rw_nop(p_cb_data cb) { (void)cb; return 0; }

static PLI_INT32 nop_vc(p_cb_data cb) {
    if (cb->value->value.integer & 1)
        at(rw_nop, cbReadWriteSynch, 0, NULL);
    return 0;
}

static int chain, chain_rises, chain_cb = cbReadWriteSynch;

static PLI_INT32 chain_link(p_cb_data cb) {
    (void)cb;
    if (chain-- > 0)
        at(chain_link, chain_cb, 0, NULL);
    return 0;
}

static PLI_INT32 chain_vc(p_cb_data cb) {
    if ((cb->value->value.integer & 1) && ++chain_rises == atoi(env("SVA_CHAIN_AT", "3"))) {
        chain = atoi(env("SVA_CHAIN", "0"));
        chain_cb = strcmp(env("SVA_CHAIN_CB", "rw"), "nba") ? cbReadWriteSynch : cbNBASynch;
        at(chain_link, chain_cb, 0, NULL);
    }
    return 0;
}

static PLI_INT32 nba_read(p_cb_data cb) {
    s_vpi_value v;
    (void)cb;
    v.format = vpiIntVal;
    vpi_get_value(vpi_handle_by_name("top.ready", NULL), &v);
    put(vpi_handle_by_name("top.seen", NULL), v.value.integer);
    return 0;
}

static PLI_INT32 nba_go(p_cb_data cb) {
    (void)cb;
    put(vpi_handle_by_name("top.go", NULL), 1);
    at(nba_read, cbNBASynch, 0, NULL);
    return 0;
}

static PLI_INT32 go_vc(p_cb_data cb) {
    if ((cb->value->value.integer & 1) && ++chain_rises == 3)
        at(nba_go, cbNBASynch, 0, NULL);
    return 0;
}

static PLI_INT32 start(p_cb_data cb) {
    const char *f = env("SVA_FLAG", "nodelay");
    (void)cb;
    cmode = env("SVA_CLK", "delay");
    dmode = env("SVA_DATA", "rw");
    flag = !strcmp(f, "inertial") ? vpiInertialDelay : !strcmp(f, "force") ? vpiForceFlag : vpiNoDelay;
    clk = vpi_handle_by_name("top.clk", NULL);
    d = vpi_handle_by_name("top.d", NULL);
    rst = vpi_handle_by_name("top.rst", NULL);
    rst_edge = atoi(env("SVA_RST", "0"));
    if (!strcmp(cmode, "vc"))
        at(tick_vc, cbValueChange, 0, vpi_handle_by_name("top.tick", NULL));
    else if (strcmp(cmode, "none"))
        at(timer, cbAfterDelay, 5, NULL);
    if (!strcmp(dmode, "vc"))
        at(clk_vc, cbValueChange, 0, clk);
    if (*env("SVA_FINISH", ""))
        at(fin_vc, cbValueChange, 0, clk);
    if (rst_edge)
        at(rst_vc, cbValueChange, 0, clk);
    if (*env("SVA_GLITCH", ""))
        at(glitch_vc, cbValueChange, 0, clk);
    if (*env("SVA_RWNOP", ""))
        at(nop_vc, cbValueChange, 0, clk);
    if (*env("SVA_CHAIN", ""))
        at(chain_vc, cbValueChange, 0, clk);
    if (*env("SVA_GO", ""))
        at(go_vc, cbValueChange, 0, clk);
    return 0;
}

static void boot(void) { at(start, cbStartOfSimulation, 0, NULL); }

void (*vlog_startup_routines[])(void) = {boot, 0};
"#;

/// A flop `q <= d` (and `q2 <= q`) with properties that hold only when `q`
/// is sampled before the edge that updates it. With `+same`, `d` changes in
/// the rising edge's own Pre-Active callback, so its sampled value is the
/// old one and `q == d` holds instead. `c_eval` counts the ticks, checks
/// each runs in the slot of its edge and prints its time.
const TOP_SV: &str = r#"
module top;
  logic clk = 0, d = 0, rst = 1, tick = 0, q = 0, q2 = 0;
  bit same;
  int fails = 0, evals = 0, late = 0;
  time tpos = 0;
  initial same = $test$plusargs("same");
  initial #203 $finish;
  always #5 tick = ~tick;
  always_ff @(posedge clk) begin q <= d; q2 <= q; end
  always @(posedge clk) tpos = $time;
  a_past: assert property (@(posedge clk) disable iff (rst || same) q == $past(d))
    else begin fails++; $display("a_past t=%0t", $time); end
  a_next: assert property (@(posedge clk) disable iff (rst || same) d |=> q)
    else begin fails++; $display("a_next t=%0t", $time); end
  a_nnext: assert property (@(posedge clk) disable iff (rst || same) !d |=> !q)
    else begin fails++; $display("a_nnext t=%0t", $time); end
  a_rose: assert property (@(posedge clk) disable iff (rst || same) $rose(q) |-> $past(d) && !$past(q))
    else begin fails++; $display("a_rose t=%0t", $time); end
  a_fell: assert property (@(posedge clk) disable iff (rst || same) $fell(q) |-> !$past(d) && $past(q))
    else begin fails++; $display("a_fell t=%0t", $time); end
  a_stable: assert property (@(posedge clk) disable iff (rst || same) $stable(q) |-> $past(d) == $past(q))
    else begin fails++; $display("a_stable t=%0t", $time); end
  a_seq: assert property (@(posedge clk) disable iff (rst || same) d ##1 !d |=> !q)
    else begin fails++; $display("a_seq t=%0t", $time); end
  a_two: assert property (@(posedge clk) disable iff (rst || same) d |-> ##2 q2)
    else begin fails++; $display("a_two t=%0t", $time); end
  a_same: assert property (@(posedge clk) disable iff (rst || !same) q == d)
    else begin fails++; $display("a_same t=%0t", $time); end
  c_eval: cover property (@(posedge clk) 1) begin
    evals++;
    if ($time != tpos) late++;
    $display("E t=%0t", $time);
  end
  final $display("RESULT evals=%0d fails=%0d late=%0d", evals, fails, late);
endmodule
"#;

/// One attempt per rising edge (5, 15, ..., 95 ns), each passing at the next
/// edge unless `disable iff (rst)` cancels it, and a cover that counts the
/// edges against the design's own count. The clock is the driver's, or with
/// `+hdl` the design's own, toggling every 5 ns.
const EDGE_SV: &str = r#"
module top;
  logic clk = 0, d = 0, rst = 1, tick = 0;
  int passes = 0, fails = 0, rises = 0, hits = 0;
  initial if ($test$plusargs("hdl")) begin #5 clk = 1; forever #5 clk = ~clk; end
  always #5 tick = ~tick;
  always @(posedge clk) rises++;
  a_rel: assert property (@(posedge clk) disable iff (rst) 1'b1 |=> 1'b1) passes++;
    else fails++;
  c_hit: cover property (@(posedge clk) 1) hits++;
  initial #100 $finish;
  final $display("RESULT passes=%0d fails=%0d rises=%0d hits=%0d", passes, fails, rises, hits);
endmodule
"#;

/// A single-cycle attempt per rising edge, and a clocking-event waiter that
/// raises `res` at the third rise (25 ns) and lowers it at the fourth: a
/// write after that edge's Observed region (§14.13, §4.4.2).
const CB_SV: &str = r#"
module top;
  logic clk = 0, d = 0, rst = 0, tick = 0, res = 0;
  int passes = 0;
  initial if ($test$plusargs("hdl")) begin #5 clk = 1; forever #5 clk = ~clk; end
  always #5 tick = ~tick;
  clocking cb @(posedge clk); endclocking
  a_now: assert property (@(posedge clk) disable iff (res) 1'b1) begin
    passes++;
    $display("P t=%0t", $time);
  end
  initial begin
    repeat (3) @(cb);
    res = 1;
    @(cb);
    res = 0;
  end
  initial #100 $finish;
  final $display("RESULT passes=%0d", passes);
endmodule
"#;

/// `go` rises from a cbNBASynch at the third clock rise (`SVA_GO`); the
/// process it wakes forks a child that sets `ready` after `#0`. A second
/// cbNBASynch, queued by the first, copies `ready` to `seen`.
const GO_SV: &str = r#"
module top;
  logic clk = 0, d = 0, rst = 0, tick = 0, go = 0, ready = 0, seen = 0;
  int hits = 0;
  initial if ($test$plusargs("hdl")) begin #5 clk = 1; forever #5 clk = ~clk; end
  always #5 tick = ~tick;
  c_hit: cover property (@(posedge clk) 1) hits++;
  always @(posedge go) fork begin #0 ready = 1; end join_none
  initial #100 $finish;
  final $display("RESULT hits=%0d seen=%0d", hits, seen);
endmodule
"#;

/// A scratch directory holding the compiled driver and `sv` as `top.sv`.
fn setup(tag: &str, sv: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("xezim_vpi_sva_{}_{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let c = dir.join("drv.c");
    let so = dir.join("drv.so");
    std::fs::write(&c, DRIVER_C).unwrap();
    std::fs::write(dir.join("top.sv"), sv).unwrap();
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

/// Run `top.sv` with the driver under `env`; returns the combined output.
fn run(dir: &Path, so: &Path, env: &[(&str, &str)], extra: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .arg("--vpi-lib")
        .arg(so)
        .args(["--no-cache", "-s", "top", "top.sv"])
        .args(extra)
        .envs(env.iter().copied())
        .current_dir(dir)
        .output()
        .expect("failed to run xezim");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// Every clock routine, data routine and write flag: 20 ticks, each in its
/// own slot, and no failure. A value written with an inertial delay in the
/// same slot as the edge it should miss races the edge, so `same` runs only
/// with the immediate flags.
#[test]
fn vpi_driven_clock_samples_preponed_values() {
    let (dir, so) = setup("matrix", TOP_SV);
    let mut bad = Vec::new();
    for clk in ["delay", "rw", "vc"] {
        for data in ["rw", "delay", "vc", "same"] {
            for flag in ["nodelay", "inertial", "force"] {
                if data == "same" && flag == "inertial" {
                    continue;
                }
                let env = [("SVA_CLK", clk), ("SVA_DATA", data), ("SVA_FLAG", flag)];
                let extra: &[&str] = if data == "same" { &["+same"] } else { &[] };
                let text = run(&dir, &so, &env, extra);
                if !text.contains("RESULT evals=20 fails=0 late=0") {
                    bad.push(format!("clk={clk} data={data} flag={flag}:\n{text}"));
                }
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// A routine that ends the run from the clock's value-change callback, as
/// cocotb does when a test returns on `RisingEdge`, ends it in that delta:
/// the assertions do not tick at that edge. `vpi_control(vpiFinish)` runs
/// `$finish` when the routine returns (§20.2), before the delta's Observed
/// region (§4.4.2), so the ticks are at 5 to 85 and not at 95.
#[test]
fn vpi_finish_at_an_edge_ticks_no_assertion() {
    let (dir, so) = setup("finish", TOP_SV);
    let env = [("SVA_CLK", "rw"), ("SVA_DATA", "rw"), ("SVA_FINISH", "95")];
    let text = run(&dir, &so, &env, &[]);
    let _ = std::fs::remove_dir_all(&dir);
    let ticks: Vec<&str> = text.lines().filter(|l| l.starts_with("E t=")).collect();
    let want: Vec<String> = (0..9).map(|k| format!("E t={}", 5 + 10 * k)).collect();
    assert_eq!(ticks, want, "{text}");
}

/// §16.12, §4.4.2: `disable iff` reads current values from the attempt's
/// start in the Observed region, which follows the whole active region set,
/// cbReadWriteSynch included. A reset released from a cbReadWriteSynch
/// queued at the second rising edge (15 ns), as cocotb writes after
/// `RisingEdge`, therefore does not cancel that edge's attempt: attempts from
/// 15 to 85 ns pass, with each clock.
#[test]
fn vpi_reset_release_at_an_edge_keeps_its_attempt() {
    let (dir, so) = setup("reset", EDGE_SV);
    let mut bad = Vec::new();
    for clk in ["delay", "rw", "vc", "none"] {
        let env = [("SVA_CLK", clk), ("SVA_RST", "2")];
        let extra: &[&str] = if clk == "none" { &["+hdl"] } else { &[] };
        let text = run(&dir, &so, &env, extra);
        if !text.contains("RESULT passes=8 fails=0 rises=10 hits=10") {
            bad.push(format!("clk={clk}:\n{text}"));
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// §4.4.2: the Observed region comes before the Reactive work of its slot,
/// whether or not a cbReadWriteSynch is due there. The attempt at 25 ns
/// passes before the clocking-event waiter raises `res` at that edge, and
/// the one at 35 ns is disabled; a no-op cbReadWriteSynch queued at each
/// rise must not hold the assertions back until after the waiter. The same
/// holds for each clock, with and without it.
#[test]
fn vpi_synch_routine_keeps_observed_before_reactive() {
    let (dir, so) = setup("reactive", CB_SV);
    let mut bad = Vec::new();
    let want: Vec<String> = [5, 15, 25, 45, 55, 65, 75, 85, 95]
        .iter()
        .map(|t| format!("P t={t}"))
        .collect();
    for clk in ["delay", "rw", "vc", "none"] {
        for nop in ["", "1"] {
            let env = [("SVA_CLK", clk), ("SVA_RWNOP", nop)];
            let extra: &[&str] = if clk == "none" { &["+hdl"] } else { &[] };
            let text = run(&dir, &so, &env, extra);
            let got: Vec<&str> = text.lines().filter(|l| l.starts_with("P t=")).collect();
            if got != want {
                bad.push(format!("clk={clk} nop={nop}:\n{text}"));
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// A cbReadWriteSynch chain at an assertion clock edge (each routine queues
/// the next, 100,000 in all, at the third rise) runs before the edge's
/// assertions one batch after another, not by recursion: the run completes
/// in an 8 MiB stack, and the attempts from the second rise on pass as in
/// the reset test (§4.4.2: the batches all precede the Observed region).
#[test]
fn vpi_synch_chain_at_an_edge_runs_iteratively() {
    let (dir, so) = setup("chain", EDGE_SV);
    let env = [
        ("SVA_CLK", "none"),
        ("SVA_RST", "2"),
        ("SVA_CHAIN", "100000"),
        ("XEZIM_STACK_MB", "8"),
    ];
    let text = run(&dir, &so, &env, &["+hdl"]);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        text.contains("RESULT passes=8 fails=0 rises=10 hits=10"),
        "{text}"
    );
}

/// §4.4.2, §4.4.3.3: the cbNBASynch routines of a slot belong to its active
/// region set too. At the second rise, where a cbReadWriteSynch releases
/// `rst`, a cbNBASynch chain of 1,000 routines (each queues the next) is due
/// as well. The assertions wait for the chain and the release, so the
/// attempts from 15 ns on pass, with each clock.
#[test]
fn vpi_nba_synch_chain_before_reset_release_keeps_its_attempt() {
    let (dir, so) = setup("nbachain", EDGE_SV);
    let mut bad = Vec::new();
    for clk in ["delay", "rw", "vc", "none"] {
        let env = [
            ("SVA_CLK", clk),
            ("SVA_RST", "2"),
            ("SVA_CHAIN", "1000"),
            ("SVA_CHAIN_CB", "nba"),
            ("SVA_CHAIN_AT", "2"),
        ];
        let extra: &[&str] = if clk == "none" { &["+hdl"] } else { &[] };
        let text = run(&dir, &so, &env, extra);
        if !text.contains("RESULT passes=8 fails=0 rises=10 hits=10") {
            bad.push(format!("clk={clk}:\n{text}"));
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// §4.4.2, §4.4.3.3: a cbNBASynch routine runs once the Active and Inactive
/// regions are empty, also while the assertions of a clock edge wait for
/// the slot's synch routines. The second routine sees the `#0` write of the
/// child the first one's write forks, with each clock, as without an
/// assertion clock event.
#[test]
fn vpi_nba_synch_follows_inactive_work() {
    let (dir, so) = setup("nbaorder", GO_SV);
    let mut bad = Vec::new();
    for clk in ["delay", "rw", "vc", "none"] {
        let env = [("SVA_CLK", clk), ("SVA_GO", "1")];
        let extra: &[&str] = if clk == "none" { &["+hdl"] } else { &[] };
        let text = run(&dir, &so, &env, extra);
        if !text.contains("RESULT hits=10 seen=1") {
            bad.push(format!("clk={clk}:\n{text}"));
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// §16.5, §4.4.2: a clock event starts the assertion's attempt even when a
/// cbReadWriteSynch routine of the same slot moves the clock back before the
/// Observed region. The rise is recorded when it happens and evaluated after
/// those routines, with the design's own clock (19 rises) and with the
/// driver's cbReadWriteSynch clock (10 rises; `rst` falls at 40 ns, so the
/// attempts from 45 to 85 ns pass).
#[test]
fn vpi_clock_moved_back_in_its_slot_keeps_its_event() {
    let (dir, so) = setup("glitch", EDGE_SV);
    let mut bad = Vec::new();
    let runs = [
        ("none", "RESULT passes=0 fails=0 rises=19 hits=19"),
        ("rw", "RESULT passes=5 fails=0 rises=10 hits=10"),
    ];
    for (clk, want) in runs {
        let env = [("SVA_CLK", clk), ("SVA_GLITCH", "1")];
        let extra: &[&str] = if clk == "none" { &["+hdl"] } else { &[] };
        let text = run(&dir, &so, &env, extra);
        if !text.contains(want) {
            bad.push(format!("clk={clk}:\n{text}"));
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}
