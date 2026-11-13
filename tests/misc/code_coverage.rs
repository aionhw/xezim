//! Code coverage (`--code-coverage`, docs/coverage-guide.md): statement,
//! branch and toggle counts in xezim_cov.json, per the guide's counting
//! rules. Where a rule matches the reference simulator's report on the same
//! design, the test says so.
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn xezim() -> String {
    // Cargo names the binary it built for the integration tests.
    env!("CARGO_BIN_EXE_xezim").to_string()
}

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("xezim_code_cov_{}_{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Run xezim on `src` (written to `t.sv` in `dir`) with `args`; the results
/// file is `cov.json` there.
fn run(dir: &Path, src: &str, args: &[&str]) -> Output {
    std::fs::write(dir.join("t.sv"), src).unwrap();
    Command::new(xezim())
        .current_dir(dir)
        .env("XEZIM_COV_DB", dir.join("cov.json"))
        .env_remove("XEZIM_CODE_COVERAGE")
        .env_remove("XEZIM_VERBOSE")
        .args(args)
        .arg("t.sv")
        .output()
        .expect("run xezim")
}

fn results(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("cov.json")).unwrap_or_default()
}

fn stmt(line: u32, kind: &str, count: u64) -> String {
    format!(
        "{{\"file\": \"t.sv\", \"line\": {}, \"kind\": \"{}\", \"count\": {}}}",
        line, kind, count
    )
}

/// The counts of the arms of the branch at `line` of kind `kind`.
fn arm_counts(json: &str, line: u32, kind: &str) -> Vec<u64> {
    let key = format!("\"line\": {}, \"kind\": \"{}\", \"arms\": [", line, kind);
    let start = json
        .find(&key)
        .unwrap_or_else(|| panic!("no {kind} at {line}: {json}"));
    let rest = &json[start + key.len()..];
    let arms = &rest[..rest.find("]}").unwrap()];
    arms.split("\"count\": ")
        .skip(1)
        .map(|c| {
            c.chars()
                .take_while(|ch| ch.is_ascii_digit())
                .collect::<String>()
                .parse()
                .unwrap()
        })
        .collect()
}

const DESIGN: &str = "\
module sub(input logic clk, input logic [1:0] sel, output logic [3:0] y);
  logic [3:0] r;
  assign y = r;
  always @(posedge clk) begin
    case (sel)
      2'd0: r <= 4'h1;
      2'd1: r <= 4'h2;
      2'd2: r <= 4'h4;
    endcase
  end
endmodule
module tb;
  logic clk = 0;
  logic [1:0] sel;
  logic [3:0] y0;
  int cnt;
  sub u0(.clk(clk), .sel(sel), .y(y0));
  always #5 clk = ~clk;
  function int f(int x);
    if (x > 2)
      return 1;
    return 0;
  endfunction
  task t(input int n);
    for (int i = 0; i < n; i++)
      cnt++;
  endtask
  initial begin
    sel = 0; cnt = 0;
    if (cnt == 5) cnt = 2; else cnt = 3;
    t(3);
    cnt = f(cnt) ? 7 : 8;
    unique case (cnt)
      7: cnt = 1;
      8: cnt = 2;
    endcase
    @(negedge clk) sel = 1;
    @(negedge clk) sel = 3;
    @(negedge clk) sel = 2;
    @(negedge clk);
    $finish;
  end
endmodule
";

/// Statements count each time they start: leaves, `always` constructs,
/// timing controls and loops (once per start), continuous assignments; `if`
/// and `case` are branches, not statements. The flop's statements and the
/// function's `return`s have the reference simulator's counts.
#[test]
fn statement_counts() {
    let d = scratch("stmt");
    let out = run(&d, DESIGN, &["--code-coverage=stmt"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json = results(&d);
    assert!(json.contains("\"kinds\": [\"statement\"]"), "{json}");
    assert!(
        !json.contains("\"branches\"") && !json.contains("\"toggles\""),
        "{json}"
    );
    for (line, kind, count) in [
        (18, "always", 8),
        (18, "statement", 8),
        (21, "statement", 1),
        (22, "statement", 0),
        (25, "for", 1),
        (26, "statement", 3),
        (30, "statement", 0),
        (30, "statement", 1),
        (32, "statement", 1),
        (34, "statement", 1),
        (35, "statement", 0),
        (37, "event", 1),
        (37, "statement", 1),
        (40, "event", 1),
        (41, "statement", 1),
        (4, "always", 4),
        (6, "statement", 1),
        (7, "statement", 1),
        (8, "statement", 1),
    ] {
        assert!(
            json.contains(&stmt(line, kind, count)),
            "line {line}: {json}"
        );
    }
    assert!(json.contains("\"line\": 3, \"kind\": \"assign\""), "{json}");
    for line in [5, 20, 33] {
        assert!(
            !json.contains(&format!("\"line\": {line}, \"kind\"")),
            "{json}"
        );
    }
    assert!(
        json.contains("\"scope\": \"tb.u0\",\n        \"design_unit\": \"sub\""),
        "{json}"
    );
    let _ = std::fs::remove_dir_all(&d);
}

/// Branch arms: `if` with its implicit `else`, `if`/`else`, `?:`, a `case`
/// without `default` (an implicit arm) and a `unique case` (none). The
/// counts match the reference simulator's report for the same design.
#[test]
fn branch_arms() {
    let d = scratch("branch");
    let out = run(&d, DESIGN, &["--code-coverage=branch"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json = results(&d);
    assert!(
        json.contains(
            "{\"file\": \"t.sv\", \"line\": 20, \"kind\": \"if\", \"arms\": [\
         {\"line\": 21, \"arm\": \"if\", \"count\": 1}, \
         {\"line\": 20, \"arm\": \"else (implicit)\", \"count\": 0}]}"
        ),
        "{json}"
    );
    assert_eq!(arm_counts(&json, 30, "if"), [0, 1]);
    assert_eq!(arm_counts(&json, 32, "ternary"), [1, 0]);
    assert!(
        json.contains(
            "{\"file\": \"t.sv\", \"line\": 33, \"kind\": \"case\", \"arms\": [\
         {\"line\": 34, \"arm\": \"item\", \"count\": 1}, \
         {\"line\": 35, \"arm\": \"item\", \"count\": 0}]}"
        ),
        "{json}"
    );
    assert!(
        json.contains(
            "{\"file\": \"t.sv\", \"line\": 5, \"kind\": \"case\", \"arms\": [\
         {\"line\": 6, \"arm\": \"item\", \"count\": 1}, \
         {\"line\": 7, \"arm\": \"item\", \"count\": 1}, \
         {\"line\": 8, \"arm\": \"item\", \"count\": 1}, \
         {\"line\": 5, \"arm\": \"default (implicit)\", \"count\": 1}]}"
        ),
        "{json}"
    );
    assert!(
        json.contains("\"branch\": {\"covered\": 8, \"total\": 12, \"percent\": 66.67}"),
        "{json}"
    );
    let _ = std::fs::remove_dir_all(&d);
}

/// An `else if` chain is one branch, and a `?:` in a continuous assignment
/// is a branch too (combinational counts are evaluations, so only which arms
/// were taken is checked).
#[test]
fn branch_chain_and_assign_ternary() {
    let d = scratch("chain");
    let src = "\
module tb;
  logic a, b, c;
  logic [1:0] k;
  wire w = a ? b : c;
  always @(a, b)
    if (a)
      k = 1;
    else if (b)
      k = 2;
    else
      k = 3;
  initial begin
    a = 0; b = 0; c = 0;
    #1 b = 1;
  end
endmodule
";
    let out = run(&d, src, &["--code-coverage=branch"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json = results(&d);
    let chain = arm_counts(&json, 6, "if");
    assert_eq!(chain.len(), 3, "{json}");
    assert!(chain[0] == 0 && chain[1] > 0 && chain[2] > 0, "{json}");
    let w = arm_counts(&json, 4, "ternary");
    assert!(w[0] == 0 && w[1] > 0, "{json}");
    assert!(json.contains("\"arm\": \"else if\""), "{json}");
    let _ = std::fs::remove_dir_all(&d);
}

/// Toggles: per bit, 0->1 and 1->0 between the ends of time slots, never to
/// or from x; input ports, memories and reals untracked. As in the reference
/// simulator's report, a 0-1-0 within one time slot is no toggle and the
/// leaf's input port is not listed.
#[test]
fn toggle_counts() {
    let d = scratch("toggle");
    let src = "\
module leaf(input logic a, output logic o);
  assign o = ~a;
endmodule
module tb;
  logic x, g;
  logic [1:0] v;
  logic [7:0] mem [4];
  int unsigned n;
  real r;
  leaf u(.a(x), .o());
  initial begin
    x = 0; g = 0; v = 0; n = 0; r = 0.5;
    #1 x = 1;
    #1 g = 1; g = 0;
    #1 v = 2'b10; v = 2'b01;
    #1 v = 2'bx1;
    #1 x = 0;
    mem[0] = 8'hff;
    n = 3;
  end
endmodule
";
    let out = run(&d, src, &["--code-coverage=toggle"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json = results(&d);
    for row in [
        "{\"signal\": \"x\", \"width\": 1, \"rise\": [1], \"fall\": [1]}",
        "{\"signal\": \"g\", \"width\": 1, \"rise\": [0], \"fall\": [0]}",
        "{\"signal\": \"v\", \"width\": 2, \"rise\": [1, 0], \"fall\": [0, 0]}",
        "{\"signal\": \"o\", \"width\": 1, \"rise\": [1], \"fall\": [1]}",
    ] {
        assert!(json.contains(row), "{row}: {json}");
    }
    assert!(
        json.contains("{\"signal\": \"n\", \"width\": 32, \"rise\": [1, 1, 0,"),
        "{json}"
    );
    for name in ["mem", "r", "a", "u"] {
        assert!(
            !json.contains(&format!("\"signal\": \"{name}\"")),
            "{name}: {json}"
        );
    }
    assert!(
        json.contains("\"toggle\": {\"covered\": 7, \"total\": 74, \"percent\": 9.46}"),
        "{json}"
    );
    let _ = std::fs::remove_dir_all(&d);
}

/// The two-state executors take back a count when they hand a block to the
/// four-state VM, so both give the same counts (the flop reads x before
/// `d` is driven).
#[test]
fn counts_do_not_depend_on_executor() {
    let d = scratch("engine");
    let src = "\
module tb;
  logic clk = 0;
  logic [3:0] q, d;
  always #5 clk = ~clk;
  always_ff @(posedge clk)
    if (d[0]) q <= d;
    else q <= ~d;
  initial begin
    #12 d = 4'h5;
    #20 d = 4'h2;
    #20 $finish;
  end
endmodule
";
    let code = |extra: &[(&str, &str)]| {
        std::fs::write(d.join("t.sv"), src).unwrap();
        let out = Command::new(xezim())
            .current_dir(&d)
            .env("XEZIM_COV_DB", d.join("cov.json"))
            .envs(extra.iter().copied())
            .args(["--code-coverage", "t.sv"])
            .output()
            .expect("run xezim");
        assert!(out.status.success());
        let json = results(&d);
        json[json.find("\"code_coverage\"").unwrap()..].to_string()
    };
    let default = code(&[]);
    let four_state = code(&[("XEZIM_TWO_STATE", "0")]);
    assert_eq!(default, four_state);
    assert!(default.contains(&stmt(5, "always_ff", 5)), "{default}");
    let _ = std::fs::remove_dir_all(&d);
}

/// Instances and generate blocks are scopes, not signals (§23.6): the names
/// of an instance array, of an instance in a generate block and of `for`,
/// `if` and `case` generate blocks, chosen or not and nested, have no toggle
/// bins. The signals declared in a generate block count in the enclosing
/// instance, as does a signal named like a nested generate block. Verilator
/// 5.038 (`--coverage-toggle`) lists the same 20 bits for `tb` (clk, d, q, p,
/// inner, u_r[0..1].r, u_if.s, cd.c, outer[0..1].inner[0..1].t), each
/// toggled both ways, and `gw[0].w`, `gw[1].w` in `tb.u_w`.
#[test]
fn toggle_skips_instance_and_generate_names() {
    let d = scratch("scope_names");
    let src = "\
module leaf(input logic clk, input logic d, output logic q);
  always_ff @(posedge clk) q <= d;
endmodule
module wrap(input logic clk, input logic [1:0] d, output logic [1:0] q);
  for (genvar g = 0; g < 2; g++) begin : gw
    logic w;
    always_ff @(posedge clk) w <= d[g];
    assign q[g] = w;
  end
endmodule
module tb;
  logic clk = 0;
  logic [3:0] d = 0;
  logic [3:0] q;
  logic [1:0] p;
  logic inner = 0;
  leaf ua[1:0] (.clk(clk), .d(d[1:0]), .q(q[1:0]));
  wrap u_w (.clk(clk), .d(d[3:2]), .q(q[3:2]));
  for (genvar i = 0; i < 2; i++) begin : u_r
    logic r;
    always_ff @(posedge clk) r <= d[i];
    leaf u_s (.clk(clk), .d(d[i]), .q(p[i]));
  end
  if (1) begin : u_if
    logic s;
    always_ff @(posedge clk) s <= d[0];
  end else begin : u_else
    logic s;
  end
  case (2)
    1: begin : c1 logic c; end
    default: begin : cd logic c; always_ff @(posedge clk) c <= d[1]; end
  endcase
  for (genvar i = 0; i < 2; i++) begin : outer
    for (genvar j = 0; j < 2; j++) begin : inner
      logic t;
      always_ff @(posedge clk) t <= d[i + j];
    end
  end
  always #5 clk = ~clk;
  initial begin
    repeat (2) @(negedge clk) d = 4'hf;
    inner = 1;
    repeat (2) @(negedge clk) d = 4'h0;
    inner = 0;
    repeat (2) @(negedge clk);
    $finish;
  end
endmodule
";
    let out = run(&d, src, &["--code-coverage=toggle"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json = results(&d);
    // A scope's entry up to the end of its toggle rows.
    let scope = |name: &str| {
        let at = json
            .find(&format!("\"scope\": \"{name}\","))
            .unwrap_or_else(|| panic!("no {name}: {json}"));
        let rows = at + json[at..].find("\"toggles\": [").unwrap();
        json[at..rows + json[rows..].find('\n').unwrap()].to_string()
    };
    let tb = scope("tb");
    for name in [
        "ua", "u_w", "u_r", "u_s", "u_if", "u_else", "c1", "cd", "outer",
    ] {
        assert!(
            !tb.contains(&format!("\"signal\": \"{name}\"")),
            "{name}: {tb}"
        );
    }
    for name in [
        "u_r[1].r",
        "u_if.s",
        "cd.c",
        "outer[1].inner[0].t",
        "inner",
        "p",
    ] {
        assert!(
            tb.contains(&format!("\"signal\": \"{name}\", ")),
            "{name}: {tb}"
        );
    }
    assert!(
        tb.contains("\"toggle\": {\"covered\": 40, \"total\": 40, \"percent\": 100.00}"),
        "{tb}"
    );
    let wrap = scope("tb.u_w");
    for name in ["gw[0].w", "gw[1].w"] {
        assert!(
            wrap.contains(&format!("\"signal\": \"{name}\", ")),
            "{name}: {wrap}"
        );
    }
    let _ = std::fs::remove_dir_all(&d);
}

/// The VPI client of `toggle_baseline_from_vpi_at_time_0`. With
/// `T0_CASE=start`, `a` is written from cbStartOfSimulation; with
/// `T0_CASE=rw`, `b` and `v` from cbReadWriteSynch at time 0 (where cocotb
/// applies its writes). Either way all three are set at 10 and cleared at
/// 20, and the time-0 cbReadOnlySynch prints `na`, `nb` and `v`.
const TIME0_VPI: &str = r#"
#include <stdlib.h>
#include <string.h>
#include "vpi_user.h"

static void put(const char *name, int val, int flags) {
    s_vpi_value v;
    s_vpi_time t = {vpiSimTime, 0, 0, 0.0};
    v.format = vpiIntVal;
    v.value.integer = val;
    vpi_put_value(vpi_handle_by_name((PLI_BYTE8 *)name, NULL), &v, &t, flags);
}

static const char *bin(const char *name, char *buf) {
    s_vpi_value v;
    v.format = vpiBinStrVal;
    vpi_get_value(vpi_handle_by_name((PLI_BYTE8 *)name, NULL), &v);
    strncpy(buf, v.value.str, 7);
    buf[7] = 0;
    return buf;
}

static PLI_INT32 cb(p_cb_data d);

static void at(int reason, unsigned t, long tag) {
    s_cb_data c;
    s_vpi_time tm = {vpiSimTime, 0, t, 0.0};
    memset(&c, 0, sizeof c);
    c.reason = reason;
    c.cb_rtn = cb;
    c.time = &tm;
    c.user_data = (PLI_BYTE8 *)tag;
    vpi_register_cb(&c);
}

static PLI_INT32 cb(p_cb_data d) {
    long tag = (long)d->user_data;
    const char *mode = getenv("T0_CASE");
    char x[8], y[8], z[8];
    switch (tag) {
    case 0:
        if (mode && !strcmp(mode, "start"))
            put("top.a", 0, vpiNoDelay);
        else
            at(cbReadWriteSynch, 0, 1);
        at(cbReadOnlySynch, 0, 9);
        at(cbAfterDelay, 10, 2);
        at(cbAfterDelay, 20, 3);
        at(cbAfterDelay, 30, 4);
        break;
    case 1:
        put("top.b", 0, vpiInertialDelay);
        put("top.v", 0, vpiNoDelay);
        break;
    case 2:
    case 3:
        put("top.a", tag == 2, vpiNoDelay);
        put("top.b", tag == 2, vpiNoDelay);
        put("top.v", tag == 2, vpiNoDelay);
        break;
    case 4:
        vpi_control(vpiFinish, 0);
        break;
    case 9:
        vpi_printf("RO na=%s nb=%s v=%s\n", bin("top.na", x), bin("top.nb", y), bin("top.v", z));
        break;
    }
    return 0;
}

static void start(void) {
    s_cb_data c;
    memset(&c, 0, sizeof c);
    c.reason = cbStartOfSimulation;
    c.cb_rtn = cb;
    vpi_register_cb(&c);
}

void (*vlog_startup_routines[])(void) = {start, 0};
"#;

/// Build the VPI client `c` in `d` and run `src` (top `top`) with it and
/// toggle coverage, `T0_CASE` set to `case`; returns the output and the
/// results file.
fn vpi_toggles(d: &Path, c: &str, src: &str, case: &str) -> (String, String) {
    let so = d.join("t0.so");
    if !so.exists() {
        std::fs::write(d.join("t0.c"), c).unwrap();
        let include = Path::new(env!("CARGO_MANIFEST_DIR")).join("include");
        let ok = Command::new(std::env::var_os("CC").unwrap_or_else(|| "clang".into()))
            .args(["-shared", "-fPIC", "-I"])
            .arg(&include)
            .arg(d.join("t0.c"))
            .arg("-o")
            .arg(&so)
            .status()
            .expect("launch cc")
            .success();
        assert!(ok, "cc failed");
    }
    std::fs::write(d.join("t.sv"), src).unwrap();
    let _ = std::fs::remove_file(d.join("cov.json"));
    let out = Command::new(xezim())
        .current_dir(d)
        .env("XEZIM_COV_DB", d.join("cov.json"))
        .env("T0_CASE", case)
        .env_remove("XEZIM_CODE_COVERAGE")
        .arg("--vpi-lib")
        .arg(&so)
        .args(["--code-coverage=toggle", "-s", "top", "t.sv"])
        .output()
        .expect("run xezim");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{case}: {text}");
    (text, results(d))
}

/// Assert the (signal, rise, fall) counts of one-bit signals.
fn toggle_rows(json: &str, rows: &[(&str, u32, u32)], what: &str) {
    for (name, rise, fall) in rows {
        let row = format!(
            "{{\"signal\": \"{name}\", \"width\": 1, \"rise\": [{rise}], \"fall\": [{fall}]}}"
        );
        assert!(json.contains(&row), "{what}: {row}: {json}");
    }
}

/// IEEE 1800-2023 §38.36.2, §38.36.3: a value a VPI client writes at time
/// 0, from cbStartOfSimulation or from that slot's cbReadWriteSynch, belongs
/// to the time-0 slot, so it is the toggle baseline, as a value the design
/// wrote at time 0 would be: toggles compare the ends of time slots and
/// never count a change from x. Nothing in the design runs at time 0, so
/// only the VPI writes make that slot. The cbStartOfSimulation write is
/// evaluated there: the time-0 cbReadOnlySynch reads `na` = 1 (§38.36.2,
/// read-only synch at the end of the slot). A signal not written at time 0
/// starts at x and its first change is no toggle. Verilator 5.038 counts the
/// same first toggle when cocotb drives these writes.
#[test]
fn toggle_baseline_from_vpi_at_time_0() {
    let d = scratch("vpi_time0");
    let src = "\
module top(input logic a, input logic b, output logic na, output logic nb);
  logic v;
  assign na = ~a;
  assign nb = ~b;
endmodule
";
    for (case, ro, rows) in [
        (
            "start",
            "RO na=1 nb=x v=x",
            [("na", 1, 1), ("nb", 1, 0), ("v", 0, 1)],
        ),
        (
            "rw",
            "RO na=x nb=1 v=0",
            [("na", 1, 0), ("nb", 1, 1), ("v", 1, 1)],
        ),
    ] {
        let (text, json) = vpi_toggles(&d, TIME0_VPI, src, case);
        assert!(text.contains(ro), "{case}: {text}");
        toggle_rows(&json, &rows, case);
    }
    let _ = std::fs::remove_dir_all(&d);
}

/// A client whose only routine is cbStartOfSimulation (§38.36.3), in a
/// design whose first event is a clock edge at 5: `w` written there is the
/// baseline of `w` and of `nw = ~w`, so `w` rises at 5 and 25 and falls at
/// 15, and `nw` the other way round. The routine is gone once it has run,
/// so nothing VPI holds the scheduler at time 0.
#[test]
fn toggle_baseline_from_lone_start_routine() {
    let d = scratch("vpi_start_only");
    let c = r#"
#include <string.h>
#include "vpi_user.h"
static PLI_INT32 cb(p_cb_data d) {
    s_vpi_value v;
    s_vpi_time t = {vpiSimTime, 0, 0, 0.0};
    v.format = vpiIntVal;
    v.value.integer = 0;
    vpi_put_value(vpi_handle_by_name("top.w", NULL), &v, &t, vpiNoDelay);
    return 0;
}
static void start(void) {
    s_cb_data c;
    memset(&c, 0, sizeof c);
    c.reason = cbStartOfSimulation;
    c.cb_rtn = cb;
    vpi_register_cb(&c);
}
void (*vlog_startup_routines[])(void) = {start, 0};
"#;
    let src = "\
module top;
  logic clk, w, nw;
  assign nw = ~w;
  initial begin clk = 0; forever #5 clk = ~clk; end
  always @(posedge clk) w <= ~w;
  always @(negedge clk) if ($time > 25) $finish;
endmodule
";
    let (_, json) = vpi_toggles(&d, c, src, "");
    toggle_rows(&json, &[("w", 2, 1), ("nw", 1, 2)], "start only");
    let _ = std::fs::remove_dir_all(&d);
}

/// A slot can now be sampled more than once: at the end of its tick and
/// again at its end, after cbAtEndOfSimTime (§38.36.2), which is still in
/// that slot. `x` goes to 1 at 10 and a cbAtEndOfSimTime routine puts it
/// back to 0 in the same slot, so 10 is no toggle (the slot's net change is
/// none); the rise at 20 is the only one.
#[test]
fn toggle_change_undone_in_its_slot() {
    let d = scratch("vpi_undo");
    let c = r#"
#include <string.h>
#include "vpi_user.h"
static PLI_INT32 cb(p_cb_data d) {
    s_cb_data c;
    s_vpi_time t = {vpiSimTime, 0, 10, 0.0};
    if (d->reason == cbAtEndOfSimTime) {
        s_vpi_value v;
        v.format = vpiIntVal;
        v.value.integer = 0;
        vpi_put_value(vpi_handle_by_name("top.x", NULL), &v, &t, vpiNoDelay);
        return 0;
    }
    memset(&c, 0, sizeof c);
    c.reason = cbAtEndOfSimTime;
    c.cb_rtn = cb;
    c.time = &t;
    vpi_register_cb(&c);
    return 0;
}
static void start(void) {
    s_cb_data c;
    memset(&c, 0, sizeof c);
    c.reason = cbStartOfSimulation;
    c.cb_rtn = cb;
    vpi_register_cb(&c);
}
void (*vlog_startup_routines[])(void) = {start, 0};
"#;
    let src = "\
module top;
  logic x;
  initial begin x = 0; #10 x = 1; #10 x = 1; #10 $finish; end
endmodule
";
    let (_, json) = vpi_toggles(&d, c, src, "");
    toggle_rows(&json, &[("x", 1, 0)], "undone");
    let _ = std::fs::remove_dir_all(&d);
}

const SMALL: &str = "\
module sub(input logic a, output logic y);
  assign y = ~a;
  initial $display(\"sub\");
endmodule
module tb;
  int x;
  logic a = 0, y;
  sub u(.a(a), .y(y));
  initial begin
    x = 1;
    #1 if (x > 0) $display(\"x=%0d y=%b\", x, y);
    $error(\"boom\");
  end
endmodule
";

/// Without code coverage nothing changes: the output and exit status are
/// the same as with it, and no results file is written for a design with
/// no covergroup or assertion. The instance's blocks are elaborated lazily;
/// instrumenting them keeps the order they run in.
#[test]
fn off_by_default_output_unchanged() {
    let d = scratch("off");
    for extra in [&[][..], &["--error-exit"][..]] {
        let _ = std::fs::remove_file(d.join("cov.json"));
        let off = run(&d, SMALL, extra);
        assert!(!d.join("cov.json").exists());
        let mut args = extra.to_vec();
        args.push("--code-coverage");
        let on = run(&d, SMALL, &args);
        assert!(results(&d).contains("\"code_coverage\""));
        assert_eq!(off.status.code(), on.status.code());
        assert_eq!(off.stdout, on.stdout);
        assert_eq!(off.stderr, on.stderr);
    }
    let _ = std::fs::remove_dir_all(&d);
}

/// Other simulators' spellings: `+cover=<letters>` picks the kinds and warns
/// once about the ones xezim lacks; bare `+cover` and `-coverage` collect all
/// three; XEZIM_CODE_COVERAGE does what the flag does.
#[test]
fn compatible_spellings() {
    let d = scratch("compat");
    let kinds = |args: &[&str]| {
        let out = run(&d, SMALL, args);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let json = results(&d);
        let start = json.find("\"kinds\": [").unwrap() + "\"kinds\": [".len();
        let k = json[start..start + json[start..].find(']').unwrap()].to_string();
        (k, String::from_utf8_lossy(&out.stderr).into_owned())
    };
    let all = "\"statement\", \"branch\", \"toggle\"";
    let (k, err) = kinds(&["+cover=sbcf"]);
    assert_eq!(k, "\"statement\", \"branch\"");
    assert_eq!(
        err.matches(
            "Warning: +cover=sbcf: xezim has no condition (c), FSM (f) coverage; \
                     collecting statement, branch coverage"
        )
        .count(),
        1,
        "{err}"
    );
    assert_eq!(kinds(&["+cover=t"]).0, "\"toggle\"");
    assert_eq!(kinds(&["+cover"]).0, all);
    assert_eq!(kinds(&["-coverage"]).0, all);
    assert_eq!(kinds(&["-coverage", "+cover=s"]).0, "\"statement\"");
    assert_eq!(
        kinds(&["--code-coverage=branch,stmt"]).0,
        "\"statement\", \"branch\""
    );
    std::fs::write(d.join("t.sv"), SMALL).unwrap();
    let env = Command::new(xezim())
        .current_dir(&d)
        .env("XEZIM_COV_DB", d.join("cov.json"))
        .env("XEZIM_CODE_COVERAGE", "toggle")
        .arg("t.sv")
        .output()
        .expect("run xezim");
    assert!(env.status.success());
    assert!(results(&d).contains("\"kinds\": [\"toggle\"]"));
    let bad = run(&d, SMALL, &["--code-coverage=lines"]);
    assert_eq!(bad.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&bad.stderr).contains("unknown code coverage kind 'lines'"));
    let _ = std::fs::remove_dir_all(&d);
}

/// `--code-coverage-scope` keeps only the listed subtree, and `--verbose`
/// prints the totals and one line per scope.
#[test]
fn scope_limit_and_summary() {
    let d = scratch("scope");
    let out = run(
        &d,
        DESIGN,
        &[
            "--code-coverage",
            "--code-coverage-scope=tb.u0",
            "--verbose",
        ],
    );
    assert!(out.status.success());
    let json = results(&d);
    assert!(json.contains("\"scope\": \"tb.u0\""), "{json}");
    assert!(!json.contains("\"scope\": \"tb\""), "{json}");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("[COV] code coverage: statement 5/5 (100.00%), branch 4/4 (100.00%), toggle"),
        "{err}"
    );
    assert!(
        err.contains("[COV]   tb.u0 (sub): statement 5/5 (100.00%)"),
        "{err}"
    );
    let _ = std::fs::remove_dir_all(&d);
}
