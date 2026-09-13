//! Rust API integration: validate the public tracing functions.

use xezim::{
    compiler::act_trace::{ActTraceCensus, DebugFlagsInput, resolve_debug_flags},
    simulate_multi,
};

#[test]
fn resolve_debug_flags_accepts_all_members() {
    // Every `DebugConfig` member is reachable, but not all at once:
    // `--debug+all` is mutually exclusive with named queries and a
    // scope-file selection (the resolver rejects those). So two calls cover
    // the members: one for `--debug+all`, one for the named-signal form.
    let all = resolve_debug_flags(DebugFlagsInput {
        all: true,
        access_raw: None,
        query_signals: vec![],
        graph_file: None,
        scope_depths: vec![],
        auto_output: true,
        census_requested: false,
        fst_requested: None,
        default_top: Some("top".into()),
        default_graph: "trace_graph.json".into(),
        default_fst: "xezim.fst".into(),
    })
    .expect("--debug+all resolves");
    assert!(all.trace_all_signals);
    assert!(all.census, "+all implies the census");
    assert!(all.query_signals.is_empty());
    assert_eq!(all.graph_file.as_deref(), Some("trace_graph.json"));
    assert_eq!(all.fst_file.as_deref(), Some("xezim.fst"));
    // Auto-output defaults the dump scope to the top module.
    assert_eq!(all.scope_depths, vec![("top".to_string(), 0u32)]);
    assert!(all.access.is_empty());

    let named = resolve_debug_flags(DebugFlagsInput {
        all: false,
        access_raw: Some("+rw".into()),
        query_signals: vec!["top.sig".into()],
        graph_file: Some("g.json".into()),
        scope_depths: vec![("top".into(), 0)],
        auto_output: true,
        census_requested: false,
        fst_requested: None,
        default_top: Some("top".into()),
        default_graph: "trace_graph.json".into(),
        default_fst: "xezim.fst".into(),
    })
    .expect("named-signal form resolves");
    assert!(!named.trace_all_signals);
    assert_eq!(named.access, vec!["r".to_string(), "w".to_string()]);
    assert_eq!(named.query_signals, vec!["top.sig"]);
    assert_eq!(named.graph_file.as_deref(), Some("g.json"));
    assert_eq!(named.scope_depths, vec![("top".to_string(), 0u32)]);
    assert_eq!(named.fst_file.as_deref(), Some("xezim.fst"));
    assert!(named.census, "access alone implies the census");
}

#[test]
fn census_records_and_writes_deterministically() {
    let mut c1 = ActTraceCensus::new(4);
    c1.record(1, 10);
    c1.record(1, 20);
    c1.record(3, 5);

    // (id, path, width) — ids happen to match positions here, so the row is
    // the identity mapping; the writer must still resolve counters via id.
    let signals = vec![(0usize, "c", 8u32), (1, "a", 1), (2, "b", 16), (3, "d", 4)];

    let mut out1 = Vec::new();
    let mut out2 = Vec::new();
    c1.write_sidecar(&mut out1, &signals, 1e-9).unwrap();
    c1.write_sidecar(&mut out2, &signals, 1e-9).unwrap();
    assert_eq!(out1, out2, "sidecar must be byte-identical");

    // Parse back: a has change_count=2, first=10, last=20 (ns via tick_s=1e-9)
    let json = String::from_utf8(out1).unwrap();
    assert!(json.contains("\"change_count\": 2"));
    assert!(json.contains("\"first_change_ns\": 10"));
    assert!(json.contains("\"last_change_ns\": 20"));
}

#[test]
fn simulate_with_census_only_no_waveform() {
    // Purely behavioral design - no structural nets, so no FST signals unless
    // the census path is exercised.
    let src = r#"
`timescale 1ns/1ns
module top;
  logic [7:0] q = 0;
  initial begin
    #10 q = 8'h11;
    #10 q = 8'h22;
    #180 $finish;
  end
endmodule
"#;
    let sim = simulate_multi(
        &[src.to_string()],
        1_000_000,
        Some("top"),
        &[],
        &[],
        None,
        false,
        None,
        None,
        &[],
        &[],
        None,
        &[],
        0,
        u64::MAX,
        None,
        &[],
        None,
        None,
        None,
        None,
        false,
        None,
    )
    .expect("simulate");

    // Census sidecar is written even without FST when env is set
    // (this test doesn't set the env, so just verify run completes)
    assert!(sim.finished);
}

#[test]
fn debug_signals_plus_graph_file_via_cli() {
    // Integration via the real binary path (same as act_trace_cone but with
    // the canonical --debug-signals / --debug-graph-file names).
    use std::path::PathBuf;
    use std::process::Command;

    fn xezim_bin() -> PathBuf {
        let mut p = std::env::current_exe().expect("current_exe");
        p.pop();
        if p.ends_with("deps") {
            p.pop();
        }
        p.join("xezim")
    }

    let dir = std::env::temp_dir().join(format!("xz_api_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let sv = dir.join("a.sv");
    std::fs::write(
        &sv,
        r#"
`timescale 1ns/1ns
module top;
  logic a = 0, b = 0;
  logic d;
  logic e;
  logic [1:0] w;
  always_comb begin
    d = a & b;
    e = d | a;
    w = {d, e};
  end
  initial begin
    #10 a = 1;
    #10 b = 1;
    #10 a = 0;
    #10 b = 0;
    #10 $finish;
  end
endmodule
"#,
    )
    .unwrap();

    let graph = dir.join("g.json");
    let out = Command::new(xezim_bin())
        .current_dir(&dir)
        .args([
            "--simulate",
            "--max-time",
            "100",
            "-s",
            "top",
            "--debug-signals",
            "d,e,w",
            "--debug-graph-file",
        ])
        .arg(&graph)
        .arg(&sv)
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(out.status.success(), "run failed: {stderr}");
    assert!(graph.exists(), "graph sidecar missing");
    let g = std::fs::read_to_string(&graph).unwrap();
    assert!(g.contains("\"path\": \"d\""));
    assert!(g.contains("\"drivers\": [\"a\", \"b\", \"d\"]"));
}
