//! Activity-trace census + driver/load cone: end-to-end value tracking.
//!
//! Runs the real `xezim` binary on a design whose per-signal change pattern
//! is analytically computable, then asserts three layers against it:
//!
//!   1. the census JSON — per-signal change counts + first/last change times,
//!      i.e. the value-change history summarized (the values driven on each
//!      signal, tracked over time);
//!   2. the cone JSON — the RTL driver (fan-in) / load (fan-out) graph;
//!   3. the captured stderr — the `[trace]` console log confirms the same
//!      numbers a user sees on their terminal.
//!
//! The design:
//!
//!   reg a (start 0); reg b (start 0)
//!   wire d = a & b;    wire e = d | a;   wire [1:0] w = {d, e};
//!   clk toggles every 5 ns
//!   a rises @10, falls @30; b rises @20, falls @40; $finish @50.
//!
//! Expected change counts (per tick, from the shared dirty set — the same
//! stream the FST/VCD dump records):
//!
//!   signal   changes   first   last
//!   a        2         10      30
//!   b        2         20      40
//!   d = a&b  2         20      30     (rises at 20 when b=1, falls at 30 when a=0)
//!   e = d|a  2         10      30     (rises when a=1, falls when d=a=0)
//!   w={d,e}  3         10      30     (touched at 10, 20, 30)
//!   clk      9         5       45     (toggles 5..45)
//!
//! Expected cone graph (transitive):
//!   d: drivers {a,b} (writes d reads a,b); loads {e,w} (read d)
//!   e: drivers {d,a}; loads {w}

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

/// Per-invocation sequence so parallel tests in this binary never share
/// one temp dir (they all see the same process id).
static DIR_SEQ: AtomicU64 = AtomicU64::new(0);

/// Path to the `xezim` binary next to the test binary.
fn xezim_bin() -> PathBuf {
    let mut p = std::env::current_exe().expect("current_exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("xezim")
}

fn design() -> &'static str {
    r#"
module top;
  reg clk = 0;
  reg a = 0, b = 0;
  wire d = a & b;
  wire e = d | a;
  wire [1:0] w = {d, e};
  always #5 clk = ~clk;
  initial begin
    #10 a = 1;   // t=10: e goes 1
    #10 b = 1;   // t=20: d goes 1
    #10 a = 0;   // t=30: d goes 0 (e stays 1)
    #10 b = 0;   // t=40: e goes 0
    #10 $finish; // t=50
  end
endmodule
"#
}

struct Run {
    census_json: String,
    cone_json: String,
    stderr: String,
}

fn run_case() -> Run {
    let dir = std::env::temp_dir().join(format!(
        "xezim_act_trace_{}_{}",
        std::process::id(),
        DIR_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let sv = dir.join("a.sv");
    std::fs::write(&sv, design().as_bytes()).expect("write sv");
    let census = dir.join("census.json");
    let cone = dir.join("cone.json");
    let _ = std::fs::remove_file(&census);
    let _ = std::fs::remove_file(&cone);

    let out = Command::new(xezim_bin())
        .current_dir(&dir)
        .env("XEZIM_ACT_TRACE_CENSUS_FILE", &census)
        .args([
            "--simulate",
            "--max-time",
            "100",
            "-s",
            "top",
            "--debug-access=+r",
            "--debug-no-auto-output",
        ])
        .args(["--debug-signals", "d,e,w", "--debug-graph-file"])
        .arg(&cone)
        .arg(&sv)
        .output()
        .expect("run xezim");

    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let census_json = std::fs::read_to_string(&census).expect("census sidecar");
    let cone_json = std::fs::read_to_string(&cone).expect("cone sidecar");
    Run {
        census_json,
        cone_json,
        stderr,
    }
}

/// Pull `"key": value` from a JSON object body.
fn json_f64(s: &str, key: &str) -> f64 {
    let marker = format!("\"{}\": ", key);
    s.find(&marker)
        .map(|i| {
            let rest = &s[i + marker.len()..];
            let end = rest
                .find(|c: char| !c.is_ascii_digit() && c != '.' && c != '-')
                .unwrap_or(rest.len());
            rest[..end].parse().unwrap_or(f64::NAN)
        })
        .unwrap_or(f64::NAN)
}

/// Extract a signal's census row and its (changes, first, last).
fn census_signal(census: &str, path: &str) -> (f64, f64, f64) {
    let start = census
        .find(&format!("\"path\": \"{}\"", path))
        .unwrap_or_else(|| panic!("census missing signal {}", path));
    let row = &census[start..census.len().min(start + 160)];
    (
        json_f64(row, "change_count"),
        json_f64(row, "first_change_ns"),
        json_f64(row, "last_change_ns"),
    )
}

/// Parse a `"drivers": [...]` / `"loads": [...]` array from a cone row.
fn cone_list(cone: &str, path: &str, field: &str) -> Vec<String> {
    let start = cone
        .find(&format!("\"path\": \"{}\"", path))
        .unwrap_or_else(|| panic!("cone missing signal {}", path));
    let row = &cone[start..];
    let begin = row
        .find(&format!("\"{}\": [", field))
        .unwrap_or_else(|| panic!("cone row {} missing {}", path, field));
    let rest = &row[begin + format!("\"{}\": [", field).len()..];
    let end = rest.find(']').expect("closing ]");
    rest[..end]
        .split(',')
        .map(|s| s.trim().trim_matches('"').to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

#[test]
fn census_tracks_values_with_time() {
    let r = run_case();
    // The census is the per-signal value history: every signal, how many
    // times its value changed, and the first/last change time.
    assert_eq!(json_f64(&r.census_json, "total_signals"), 6.0);
    for (path, changes, first, last) in [
        ("a", 2.0, 10.0, 30.0),
        ("b", 2.0, 20.0, 40.0),
        // d = a & b rises when b arrives, falls when a leaves.
        ("d", 2.0, 20.0, 30.0),
        // e = d | a rises with a, falls when both drop.
        ("e", 2.0, 10.0, 30.0),
        // w = {d, e} touched at 10 (e), 20 (d), 30 (both) = 3.
        ("w", 3.0, 10.0, 30.0),
    ] {
        let (got_ch, got_first, got_last) = census_signal(&r.census_json, path);
        // f64 from (tick * 1e9) has ~1e-15 noise; compare with tolerance.
        let tol = 1e-9;
        assert_eq!(got_ch, changes, "{} change count", path);
        assert!(
            (got_first - first).abs() < tol,
            "{} first: {} vs {}",
            path,
            got_first,
            first
        );
        assert!(
            (got_last - last).abs() < tol,
            "{} last: {} vs {}",
            path,
            got_last,
            last
        );
    }
    // The clock's first edge is unambiguous even if the tail is not.
    let (clk_ch, clk_first, _) = census_signal(&r.census_json, "clk");
    // f64 tolerance
    assert!((clk_first - 5.0).abs() < 1e-9, "clk first change");
    assert!(clk_ch >= 9.0, "clk at least 9 toggles to 45ns");
}

#[test]
fn cone_graph_drivers_and_loads() {
    let r = run_case();
    let mut drivers: Vec<String> = cone_list(&r.cone_json, "d", "drivers");
    let mut loads: Vec<String> = cone_list(&r.cone_json, "d", "loads");
    drivers.sort();
    loads.sort();
    assert_eq!(
        &drivers,
        &["a", "b", "d"],
        "drivers of d = write-set of d's entry"
    );
    assert_eq!(&loads, &["d", "e", "w"], "loads of d = readers of d");
    // e's fan-in transits through d: a and b are both in e's driver cone.
    let e_drivers = cone_list(&r.cone_json, "e", "drivers");
    assert!(
        e_drivers.iter().any(|s| s == "a") && e_drivers.iter().any(|s| s == "b"),
        "e reaches the leaves a,b through d (transitive fan-in)"
    );
    // w is the last consumer: its loads are just itself.
    assert_eq!(cone_list(&r.cone_json, "w", "loads"), vec!["w"]);
}

#[test]
fn stderr_log_confirms_counts() {
    let r = run_case();
    // The `[trace]` console log is what a user running --debug-signals sees.
    assert!(r.stderr.contains("[trace] === d "), "log has d header");
    assert!(
        r.stderr.contains("[trace] === e (id="),
        "log has e header (id resolved)"
    );
    // The header carries the census change count for the queried signal.
    assert!(
        r.stderr.contains("changes=2"),
        "log reports changes=2 for d/e/w (got: {})",
        r.stderr
            .lines()
            .find(|l| l.contains("[trace] === "))
            .unwrap_or("")
    );
    // At least one driver line + one load line for d.
    assert!(
        r.stderr.contains("drivers (fan-in,") && r.stderr.contains("loads (fan-out,"),
        "log has both driver and load sections"
    );
}

// ---------------------------------------------------------------------------
// Type-agnostic tracing: enum/struct/logic/reg/wire/bit all trace, and the
// enum member constants never leak into the census or the cone graph.
// ---------------------------------------------------------------------------

/// Everything is a signal here: `logic`, `reg`, `wire`, `bit`, a typedef'd
/// enum, an anonymous enum, and packed structs. The LRM allows a net or
/// variable to carry any of these data types (IEEE 1800-2023 §6.8, §6.19,
/// §7.2), so the trace must treat them all alike. What must NOT appear is
/// the enum member names: `IDLE`/`RUN`/`DONE` and `E0`/`E1` are elaboration
/// constants that elaboration also mints as signal-table entries. They are
/// not signals and must be invisible to the census and the cone graph.
fn mixed_types_design() -> &'static str {
    r#"
`timescale 1ns/1ns
module top;
  typedef enum logic [1:0] { IDLE, RUN, DONE } state_t;
  state_t st  = IDLE;
  state_t lead = RUN;
  logic [1:0] a = 0, b = 0, c = 0;
  wire logic [1:0] d = a & b;
  reg [1:0] r = 0;
  bit tail = 0;
  typedef struct packed { logic [1:0] hi; logic [1:0] lo; } pair_t;
  pair_t p;
  pair_t q;
  enum { E0, E1 } anon;
  reg clk = 0;
  always #5 clk = ~clk;
  always_comb begin
    st = lead[0] ? RUN : IDLE;
    p  = '{hi: st, lo: d};
  end
  assign q = p;
  assign tail = d[0] ^ b[0];
  always @(posedge clk) r <= d;
  always_comb anon = lead[0] ? E0 : E1;
  initial begin
    #10 a = 2'b01;
    #10 b = 2'b10; lead = DONE;
    #10 a = 0;
    #10 lead = IDLE;
    #10 $finish;
  end
endmodule
"#
}

/// Run the binary on `src` with `--debug+all` and a census sidecar.
fn run_types_case() -> Run {
    let dir = std::env::temp_dir().join(format!(
        "xezim_act_trace_types_{}_{}",
        std::process::id(),
        DIR_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let sv = dir.join("a.sv");
    std::fs::write(&sv, mixed_types_design().as_bytes()).expect("write sv");
    let census = dir.join("census.json");
    let cone = dir.join("cone.json");
    let _ = std::fs::remove_file(&census);
    let _ = std::fs::remove_file(&cone);

    let out = Command::new(xezim_bin())
        .current_dir(&dir)
        .args([
            "--simulate",
            "--max-time",
            "100",
            "-s",
            "top",
            "--debug+all",
            "--debug-graph-file",
        ])
        .env("XEZIM_ACT_TRACE_CENSUS_FILE", &census)
        .arg(&cone)
        .arg(&sv)
        .output()
        .expect("run xezim");

    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(out.status.success(), "run failed:\n{stderr}");
    Run {
        census_json: std::fs::read_to_string(&census).expect("census sidecar"),
        cone_json: std::fs::read_to_string(&cone).expect("cone sidecar"),
        stderr,
    }
}

/// The real signals in `mixed_types_design`, in no particular order.
fn real_signal_set() -> Vec<&'static str> {
    vec![
        "st", "lead", "a", "b", "c", "d", "r", "tail", "p", "q", "anon", "clk",
    ]
}

/// The enum member names minted as signal-table entries but not real signals.
fn enum_member_names() -> Vec<&'static str> {
    vec!["IDLE", "RUN", "DONE", "E0", "E1"]
}

/// Every `"path": "..."` string in a sidecar.
fn json_paths(s: &str) -> Vec<String> {
    s.split("\"path\": \"")
        .skip(1)
        .map(|tail| {
            let end = tail.find('"').expect("closing quote");
            tail[..end].to_string()
        })
        .collect()
}

#[test]
fn census_and_cone_skip_enum_member_constants() {
    let r = run_types_case();
    let members = enum_member_names();

    // Census: exactly the real signals, no enum member names among the rows.
    let census_paths = json_paths(&r.census_json);
    let real = real_signal_set();
    assert_eq!(
        census_paths.len(),
        real.len(),
        "census total_signals must equal real signal count"
    );
    for m in &members {
        assert!(
            !census_paths.iter().any(|p| p == m),
            "census must not list enum member {m}: {}",
            r.census_json
        );
    }

    // The graph (--debug+all expands to every signal) has one row per real
    // signal, and none for the enum members.
    let graph_paths = json_paths(&r.cone_json);
    assert_eq!(
        graph_paths.len(),
        real.len(),
        "graph must have one row per real signal, got {} rows",
        graph_paths.len()
    );
    for m in &members {
        assert!(
            !graph_paths.iter().any(|p| p == m),
            "graph must not list enum member {m}"
        );
    }

    // No driver/load list anywhere may name an enum member.
    for m in &members {
        let needle = format!("\"{m}\"");
        assert!(
            !r.cone_json.contains(&needle),
            "cone graph must not reference enum member {m}"
        );
    }

    // Data types are preserved. The struct `pair_t` packs two 2-bit fields.
    // Counts are traced from the dump stream (change vs. the t=0 baseline):
    // `st` (the enum variable) flips RUN->IDLE once when `lead` crosses to
    // DONE at t=20; `p` (the struct) and `q` (an assign of p) both track `st`
    // through the always_comb, so each changes exactly once, at t=20.
    assert_eq!(census_signal(&r.census_json, "st").0, 1.0);
    assert_eq!(census_signal(&r.census_json, "p").0, 1.0);
    assert_eq!(census_signal(&r.census_json, "q").0, 1.0);
}

#[test]
fn enum_typed_signal_drives_struct_and_anon_enum() {
    let r = run_types_case();
    // st = f(lead); the process index groups the always_comb read set, so the
    // driver cone reaches lead (and d's drivers a,b through the sibling
    // `p = '{hi: st, lo: d}` statement), but never the enum literals.
    let st_drivers = cone_list(&r.cone_json, "st", "drivers");
    let st_loads = cone_list(&r.cone_json, "st", "loads");
    assert!(
        st_drivers.iter().any(|s| s == "lead"),
        "st is combinational from lead: {st_drivers:?}"
    );
    assert!(
        st_drivers.iter().any(|s| s == "st"),
        "st carries its own write set: {st_drivers:?}"
    );
    for m in enum_member_names() {
        assert!(
            !st_drivers.iter().any(|s| s == m),
            "st drivers must not list {m}: {st_drivers:?}"
        );
        assert!(
            !st_loads.iter().any(|s| s == m),
            "st loads must not list {m}: {st_loads:?}"
        );
    }
    for (path, _has_lead) in [("p", true), ("q", true), ("anon", true)] {
        let drivers = cone_list(&r.cone_json, path, "drivers");
        assert!(
            drivers.iter().any(|s| s == "lead"),
            "{path} fan-in should reach lead: {drivers:?}"
        );
        for m in enum_member_names() {
            assert!(
                !drivers.iter().any(|s| s == m),
                "{path} drivers must not list {m}: {drivers:?}"
            );
        }
    }
}
