//! `--debug` flag family: combination matrix.
//!
//! Two layers of coverage:
//!
//!   1. In-process: `resolve_debug_flags` is exercised over an exhaustive
//!      truth table of the flag switches, checked against an independent
//!      oracle that mirrors the documented rules. Every valid combined state
//!      yields the expected configuration, every rejected one yields an
//!      error naming the conflict and a valid fix.
//!   2. End-to-end: the real binary is run on a tiny design for the valid
//!      sets (default outputs, `--debug+all` all-signal graph, graph-only
//!      run) and for each invalid combination (exit != 0, stderr carries the
//!      conflict and a usable example). The retired `--trace-cone` name is
//!      also checked: it must fail with a pointer at the replacement.

use std::path::PathBuf;
use std::process::{Command, Output};

use xezim::compiler::act_trace::{DebugFlagsInput, resolve_debug_flags};

/// Path to the `xezim` binary next to the test binary.
fn xezim_bin() -> PathBuf {
    let mut p = std::env::current_exe().expect("current_exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("xezim")
}

/// Tiny design: six signals with analytically known values.
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
    #10 a = 1;
    #10 b = 1;
    #10 a = 0;
    #10 b = 0;
    #10 $finish;
  end
endmodule
"#
}

fn run_in(dir: &PathBuf, args: &[&str]) -> Output {
    let sv = dir.join("a.sv");
    std::fs::write(&sv, design().as_bytes()).expect("write sv");
    Command::new(xezim_bin())
        .current_dir(dir)
        .args(["--simulate", "--max-time", "100", "-s", "top"])
        .args(args)
        .arg(&sv)
        .output()
        .expect("run xezim")
}

/// Count `"path": "` occurrences in the graph sidecar (one per queried row).
fn graph_rows(graph: &str) -> usize {
    graph.matches("\"path\": \"").count()
}

// ---------------------------------------------------------------------------
// 1. Exhaustive in-process truth table against an independent oracle.
// ---------------------------------------------------------------------------

#[test]
fn resolver_exhaustive_truth_table() {
    let default_graph = "trace_graph.json";
    let default_fst = "xezim.fst";
    let mut checked = 0usize;
    for debug_all in [false, true] {
        for auto_output in [false, true] {
            for scope_present in [false, true] {
                for queries_present in [false, true] {
                    for census_requested in [false, true] {
                        for access_present in [false, true] {
                            checked += 1;
                            let scope = if scope_present {
                                vec![("top".into(), 1u32)]
                            } else {
                                vec![]
                            };
                            let queries = if queries_present {
                                vec!["d".into()]
                            } else {
                                vec![]
                            };
                            let access = if access_present { Some("+rw") } else { None };
                            let got = resolve_debug_flags(DebugFlagsInput {
                                all: debug_all,
                                access_raw: access.map(str::to_string),
                                query_signals: queries.clone(),
                                graph_file: None,
                                scope_depths: scope.clone(),
                                auto_output,
                                census_requested,
                                fst_requested: None,
                                default_top: Some("top".to_string()),
                                default_graph: default_graph.to_string(),
                                default_fst: default_fst.to_string(),
                            });
                            let tracing = debug_all || access_present || scope_present;
                            // Independent oracle: the documented rules.
                            let expected: Result<
                                (
                                    bool,
                                    bool,
                                    Vec<String>,
                                    Option<&str>,
                                    Option<&str>,
                                    Vec<(String, u32)>,
                                ),
                                &str,
                            > = if debug_all && scope_present {
                                Err("scope")
                            } else if debug_all && queries_present {
                                Err("signals")
                            } else if !tracing && !auto_output {
                                Err("no-auto")
                            } else {
                                Ok((
                                    census_requested || tracing,
                                    debug_all,
                                    if debug_all { vec![] } else { queries },
                                    if tracing && auto_output {
                                        Some(default_graph)
                                    } else {
                                        None
                                    },
                                    if tracing && auto_output {
                                        Some(default_fst)
                                    } else {
                                        None
                                    },
                                    if scope.is_empty() && tracing && auto_output {
                                        vec![("top".to_string(), 0u32)]
                                    } else {
                                        scope
                                    },
                                ))
                            };
                            match (got, expected) {
                                (Ok(cfg), Ok((census, all, qs, graph, fst, exp_scope))) => {
                                    assert_eq!(
                                        cfg.census, census,
                                        "census all={debug_all} auto={auto_output} scope={scope_present} q={queries_present} census={census_requested} acc={access_present}"
                                    );
                                    assert_eq!(cfg.trace_all_signals, all, "all-signals mismatch");
                                    assert_eq!(cfg.query_signals, qs, "queries mismatch");
                                    assert_eq!(
                                        cfg.graph_file.as_deref(),
                                        graph,
                                        "graph default mismatch"
                                    );
                                    assert_eq!(
                                        cfg.fst_file.as_deref(),
                                        fst,
                                        "fst default mismatch"
                                    );
                                    assert_eq!(
                                        cfg.scope_depths, exp_scope,
                                        "scope/default mismatch"
                                    );
                                    assert_eq!(
                                        cfg.access,
                                        if access_present {
                                            vec!["r".to_string(), "w".to_string()]
                                        } else {
                                            vec![]
                                        }
                                    );
                                }
                                (Err(msg), Err(kind)) => {
                                    assert!(!msg.is_empty());
                                    // Every rejection names the fix, not just the problem.
                                    assert!(
                                        msg.contains("Use "),
                                        "kind={} must show a valid use, got: {msg}",
                                        kind
                                    );
                                }
                                (Ok(cfg), Err(kind)) => panic!(
                                    "oracle rejected kind={kind} but resolver accepted: {cfg:?}"
                                ),
                                (Err(msg), Ok(_)) => {
                                    panic!("oracle accepted but resolver rejected: {msg}")
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    assert_eq!(checked, 64, "truth table must stay exhaustive");
}

#[test]
fn resolver_access_values() {
    let base = vec!["d".into()];
    let ok = resolve_debug_flags(DebugFlagsInput {
        all: false,
        access_raw: Some("+rw".to_string()),
        query_signals: base.clone(),
        graph_file: None,
        scope_depths: vec![],
        auto_output: true,
        census_requested: false,
        fst_requested: None,
        default_top: Some("top".to_string()),
        default_graph: "g.json".to_string(),
        default_fst: "f.fst".to_string(),
    });
    let cfg = ok.expect("+rw is valid");
    assert_eq!(cfg.access, vec!["r".to_string(), "w".to_string()]);
    assert!(cfg.census, "access alone enables the census");
    // Tokens in either spelling of the flag value are accepted.
    let all = resolve_debug_flags(DebugFlagsInput {
        all: false,
        access_raw: Some("+mem+pp".to_string()),
        query_signals: base.clone(),
        graph_file: None,
        scope_depths: vec![],
        auto_output: true,
        census_requested: false,
        fst_requested: None,
        default_top: Some("top".to_string()),
        default_graph: "g.json".to_string(),
        default_fst: "f.fst".to_string(),
    })
    .expect("+mem+pp is valid");
    assert_eq!(all.access, vec!["mem".to_string(), "pp".to_string()]);
    // Unknown token, empty value, and bare '+' are all rejected.
    for bad in ["+xyz", "", "+", "++"] {
        let err = match resolve_debug_flags(DebugFlagsInput {
            all: false,
            access_raw: Some(bad.to_string()),
            query_signals: base.clone(),
            graph_file: None,
            scope_depths: vec![],
            auto_output: true,
            census_requested: false,
            fst_requested: None,
            default_top: Some("top".to_string()),
            default_graph: "g.json".to_string(),
            default_fst: "f.fst".to_string(),
        }) {
            Err(e) => e,
            Ok(cfg) => panic!("access value {bad:?} must be rejected, got {cfg:?}"),
        };
        assert!(!err.is_empty(), "message for {bad:?}");
    }
    // A named graph file always wins over the auto default.
    let named = resolve_debug_flags(DebugFlagsInput {
        all: false,
        access_raw: None,
        query_signals: base.clone(),
        graph_file: Some("mine.json".to_string()),
        scope_depths: vec![],
        auto_output: true,
        census_requested: false,
        fst_requested: None,
        default_top: Some("top".to_string()),
        default_graph: "g.json".to_string(),
        default_fst: "f.fst".to_string(),
    })
    .expect("named graph file is valid");
    assert_eq!(named.graph_file.as_deref(), Some("mine.json"));
    // A user FST path wins over the auto default.
    let named_fst = resolve_debug_flags(DebugFlagsInput {
        all: true,
        access_raw: None,
        query_signals: vec![],
        graph_file: None,
        scope_depths: vec![],
        auto_output: true,
        census_requested: false,
        fst_requested: Some("u.fst".to_string()),
        default_top: Some("top".to_string()),
        default_graph: "g.json".to_string(),
        default_fst: "f.fst".to_string(),
    })
    .expect("named fst is valid");
    assert_eq!(named_fst.fst_file.as_deref(), Some("u.fst"));
}

// ---------------------------------------------------------------------------
// 2. End-to-end: valid sets produce the right artifacts.
// ---------------------------------------------------------------------------

#[test]
fn e2e_debug_default_outputs() {
    let dir = std::env::temp_dir().join(format!("xz_flags_debug_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    let out = run_in(&dir, &["--debug-access=+r"]);
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(out.status.success(), "run must succeed:\n{stderr}");
    // All three defaults appear in the run directory.
    for f in ["xezim.fst", "trace_census.json", "trace_graph.json"] {
        let p = dir.join(f);
        assert!(p.exists(), "{f} must exist for --debug (stderr: {stderr})");
        assert!(
            std::fs::metadata(&p).unwrap().len() > 0,
            "{f} must be non-empty"
        );
    }
    let graph = std::fs::read_to_string(dir.join("trace_graph.json")).unwrap();
    assert_eq!(
        graph_rows(&graph),
        0,
        "--debug-access alone: no query signals, empty graph"
    );
}

#[test]
fn e2e_debug_all_explores_every_signal() {
    let dir = std::env::temp_dir().join(format!("xz_flags_all_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    let out = run_in(&dir, &["--debug+all"]);
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(out.status.success(), "run must succeed:\n{stderr}");
    // Six signals in the design; --debug+all expands the graph to all of them.
    let graph = std::fs::read_to_string(dir.join("trace_graph.json")).unwrap();
    assert_eq!(
        graph_rows(&graph),
        6,
        "--debug+all must graph every signal (stderr: {stderr})"
    );
    // a has no drivers; it is only read. w has no loads; it is the last consumer.
    assert!(graph.contains("\"path\": \"a\""), "a present");
    assert!(graph.contains("\"path\": \"w\""), "w present");
}

#[test]
fn e2e_graph_only_without_debug() {
    let dir = std::env::temp_dir().join(format!("xz_flags_graph_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    let out = run_in(
        &dir,
        &["--debug-signals", "d,e,w", "--debug-graph-file", "g.json"],
    );
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        out.status.success(),
        "graph-only run must succeed:\n{stderr}"
    );
    // Without --debug there are no automatic dump defaults.
    assert!(
        !dir.join("xezim.fst").exists(),
        "no default FST without --debug"
    );
    assert!(
        !dir.join("trace_census.json").exists(),
        "no default census without --debug"
    );
    // The graph sidecar is named and populated.
    let graph = std::fs::read_to_string(dir.join("g.json")).unwrap();
    assert_eq!(graph_rows(&graph), 3, "d, e, w rows (stderr: {stderr})");
    assert!(
        graph.contains("\"drivers\": [\"a\", \"b\", \"d\"],"),
        "d's driver cone (stderr: {stderr})"
    );
}

#[test]
fn e2e_scope_file_with_debug() {
    let dir = std::env::temp_dir().join(format!("xz_flags_scope_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(
        dir.join("scopes.txt"),
        "flat, top, A\nflat, top.a, A\nflat, top.b, A\n",
    )
    .expect("write scopes");
    let out = run_in(&dir, &["--debug-scope-file", "scopes.txt"]);
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        out.status.success(),
        "standalone scope-file run must succeed:\n{stderr}"
    );
    assert!(dir.join("xezim.fst").exists());
    let census = std::fs::read_to_string(dir.join("trace_census.json")).unwrap();
    // The selected flat scope set (3 top-level scopes) is what got dumped.
    assert!(
        census.contains("\"path\": \"a\"") || census.contains("\"path\": \"top.a\""),
        "selected signal present"
    );
}

// ---------------------------------------------------------------------------
// 3. End-to-end: invalid combinations exit non-zero with a usable fix.
// ---------------------------------------------------------------------------

fn expect_rejected(args: &[&str], needle: &str, expect_example: bool) {
    let dir = std::env::temp_dir().join(format!(
        "xz_flags_bad_{}_{}",
        needle.replace(' ', "_").replace('+', "p"),
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    let out = run_in(&dir, args);
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        !out.status.success(),
        "must exit non-zero for {args:?}:\n{stderr}"
    );
    assert!(
        stderr.contains(needle),
        "stderr must mention '{needle}' for {args:?} (got:\n{stderr})"
    );
    if expect_example {
        let fix_line = stderr
            .lines()
            .find(|l| l.contains("Valid:") || l.contains("Use "));
        assert!(
            fix_line.is_some(),
            "error must carry a valid example for {args:?}:\n{stderr}"
        );
    }
}

#[test]
fn e2e_invalid_combinations_are_rejected() {
    // +all vs a signal subset.
    expect_rejected(
        &["--debug+all", "--debug-signals", "d"],
        "cannot be combined",
        true,
    );
    // +all vs a scope-file selection.
    let dir = std::env::temp_dir().join(format!("xz_flags_bad_scope_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(dir.join("scopes.txt"), "deep, top, A\n").expect("write scopes");
    let out = run_in(&dir, &["--debug+all", "--debug-scope-file", "scopes.txt"]);
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(!out.status.success(), "+all + scope-file must fail");
    assert!(
        stderr.contains("cannot be combined"),
        "+all + scope-file: {stderr}"
    );
    assert!(
        stderr.lines().any(|l| l.contains("Use ")),
        "+all + scope-file must show the fix: {stderr}"
    );
    // no-auto-output with no dump-enabling debug flag.
    expect_rejected(&["--debug-no-auto-output"], "needs a", true);
    // The bare --debug prefix is not a flag; it points at the family.
    expect_rejected(&["--debug"], "implicit", true);
    // Invalid access token.
    expect_rejected(&["--debug+all", "--debug-access=+xyz"], "not valid", true);
    // Retired cone names point at the replacement.
    expect_rejected(&["--trace-cone", "d"], "retired", true);
    expect_rejected(&["--cone-file", "x.json"], "retired", true);
}
