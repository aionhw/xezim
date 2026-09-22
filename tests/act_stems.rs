//! `.stems` RTLBrowse companion file: hierarchy + source-position sidecar.
//!
//! Runs the real `xezim` binary on hierarchical designs, then validates the
//! emitted `.stems` file against the exact grammar GTKWave's `rtlbrowse`
//! consumes (`load_stems_file` in `contrib/rtlbrowse/stem_recurse.c`):
//!
//!   ++ module <name> file <path> lines <start> - <end>
//!   ++ comp <instance> type <module-name> parent <parent-module-name>
//!
//! The assertions mirror what rtlbrowse actually does with the file: every
//! `++ comp` type/parent must name a module that has a `++ module` record
//! (else the tree shows it as `[MISSING]`), and the top module — referenced
//! by no comp — must be the single tree root (rtlbrowse keys its roots on
//! `refcnt == 0`).
//!
//! Coverage is deliberately broad: declaration-line parity across header
//! shapes (plain, ANSI, parameterized, escaped, attribute-decorated),
//! hierarchy depth and instance-name collisions, multi-file designs,
//! repeated instantiation, forward references, interface instances, and the
//! documented degraded modes (compiled artifacts carry no preprocessed text;
//! a whitespace-containing path breaks the `%s` grammar).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

/// Per-invocation sequence so parallel tests in this binary never share one
/// temp dir (they all see the same process id).
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

/// A fresh scratch dir unique to this invocation.
fn new_scratch(kind: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "xezim_act_stems_{}_{}_{}",
        kind,
        std::process::id(),
        DIR_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("mkdir");
    dir
}

/// Run the real binary; return (success, stderr).
fn invoke_xezim(dir: &Path, args: &[String]) -> (bool, String) {
    let out = Command::new(xezim_bin())
        .current_dir(dir)
        .args(args)
        .output()
        .expect("run xezim");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

struct Run {
    stems: String,
    stderr: String,
}

/// Run `xezim` on one or more sources (`(label, text)` pairs written as
/// `<label>.sv`, first pair passed first on the command line) with
/// `--stems <dir>/a.stems` and an optional FST dump; assert the run
/// succeeded and the sidecar was written, and return it.
fn run_case_multi(files: &[(&str, &str)], fst: bool) -> Run {
    let dir = new_scratch("multi");
    let mut sv_paths: Vec<PathBuf> = Vec::new();
    for (label, text) in files {
        let p = dir.join(format!("{label}.sv"));
        std::fs::write(&p, text).expect("write sv");
        sv_paths.push(p);
    }
    let stems = dir.join("a.stems");
    let _ = std::fs::remove_file(&stems);
    let mut args: Vec<String> = vec![
        "--simulate".to_string(),
        "--max-time".to_string(),
        "100".to_string(),
        "-s".to_string(),
        "top".to_string(),
        "--stems".to_string(),
        stems.display().to_string(),
    ];
    if fst {
        args.push("--fst".to_string());
        args.push(dir.join("a.fst").display().to_string());
    }
    args.extend(sv_paths.iter().map(|p| p.display().to_string()));
    let (ok, stderr) = invoke_xezim(&dir, &args);
    assert!(ok, "xezim run failed:\n{stderr}");
    let stems_text = std::fs::read_to_string(&stems).expect("stems sidecar");
    Run {
        stems: stems_text,
        stderr,
    }
}

fn run_case(sv: &str, fst: bool) -> Run {
    run_case_multi(&[("a", sv)], fst)
}

/// Run a design that must FAIL elaboration, and report stderr plus whether a
/// sidecar was nevertheless left behind (it must not be).
fn run_case_failure(sv: &str) -> (String, bool) {
    let dir = new_scratch("fail");
    let sv_path = dir.join("a.sv");
    std::fs::write(&sv_path, sv).expect("write sv");
    let stems = dir.join("a.stems");
    let _ = std::fs::remove_file(&stems);
    let args: Vec<String> = vec![
        "--simulate".to_string(),
        "--max-time".to_string(),
        "100".to_string(),
        "-s".to_string(),
        "top".to_string(),
        "--stems".to_string(),
        stems.display().to_string(),
        sv_path.display().to_string(),
    ];
    let (ok, stderr) = invoke_xezim(&dir, &args);
    assert!(!ok, "expected the run to fail");
    (stderr, stems.exists())
}

/// A three-level hierarchy: `sub` instantiated twice (once at the top, once
/// under an instance of `mid`). Exercises comp records at every level,
/// including a parent module (`mid`) that is itself only an instance.
fn design() -> &'static str {
    r#"module sub;
  reg s;
  always #7 s = ~s;
endmodule

module mid;
  reg m;
  sub u3 ();
  always #3 m = ~m;
endmodule

module top;
  sub u1 ();
  mid u2 ();
endmodule
"#
}

/// A parsed `.stems` record, matching the `++ ` grammar rtlbrowse reads.
#[derive(Debug, PartialEq)]
enum Rec {
    Module {
        name: String,
        file: String,
        start: u32,
        end: u32,
    },
    Comp {
        instance: String,
        mtype: String,
        parent: String,
    },
}

/// Re-implementation of the rtlbrowse parser (`load_stems_file`): tokenize
/// the fixed `++ keyword ...` lines with whitespace splitting, the exact
/// shape `xml2stems` emits and `sscanf("%s %s ...")` consumes.
fn parse_stems(text: &str) -> Vec<Rec> {
    let mut out = Vec::new();
    for line in text.lines() {
        if !(line.starts_with("++ ")) {
            continue;
        }
        let tok: Vec<&str> = line.split_whitespace().collect();
        match tok[1] {
            "module" => {
                // ++ module <name> file <file> lines <s> - <e>
                assert_eq!(tok[3], "file", "module record shape");
                assert_eq!(tok[5], "lines", "module record shape");
                assert_eq!(tok[7], "-", "module record shape");
                let name = parse_name(tok[2]);
                let s: u32 = tok[6].parse().unwrap();
                let e: u32 = tok[8].parse().unwrap();
                assert!(s >= 1 && e >= 1, "lines are 1-based");
                out.push(Rec::Module {
                    name,
                    file: tok[4].to_string(),
                    start: s,
                    end: e,
                });
            }
            "comp" => {
                // ++ comp <instance> type <module> parent <parent>
                assert_eq!(tok[3], "type", "comp record shape");
                assert_eq!(tok[5], "parent", "comp record shape");
                out.push(Rec::Comp {
                    instance: parse_name(tok[2]),
                    mtype: parse_name(tok[4]),
                    parent: parse_name(tok[6]),
                });
            }
            other => panic!("unexpected stems directive '{}'", other),
        }
    }
    out
}

/// rtlbrowse truncates identifiers at a ':' (its generate-block hack in
/// `recurse_into_modules`/`compar_comp_array_bsearch`); mirror that so a
/// readback comparison is apples-to-apples.
fn parse_name(tok: &str) -> String {
    let cut = tok.find(':').unwrap_or(tok.len());
    tok[..cut].to_string()
}

/// What rtlbrowse's `create_module` needs to hold: the set of module
/// definitions, and for each comp that its type and parent both name a
/// module record (otherwise the tree item would render `[MISSING]`).
fn validate(recs: &[Rec]) {
    let mut modules: Vec<&String> = recs
        .iter()
        .filter_map(|r| match r {
            Rec::Module { name, .. } => Some(name),
            _ => None,
        })
        .collect();
    // Into a sorted set for the root-count logic below.
    modules.sort();
    modules.dedup();

    let mut roots = 0usize;
    for m in modules.iter() {
        // Modules never referenced as a comp TYPE are rtlbrowse roots.
        let referenced = recs.iter().any(|r| match r {
            Rec::Comp { mtype, .. } => mtype.as_str() == m.as_str(),
            _ => false,
        });
        if !referenced {
            roots += 1;
        }
    }
    assert_eq!(roots, 1, "exactly one tree root (the top module)");

    for r in recs {
        if let Rec::Comp {
            instance,
            mtype,
            parent,
        } = r
        {
            assert!(
                modules.contains(&mtype),
                "comp '{}' instantiates unknown module '{}'",
                instance,
                mtype
            );
            assert!(
                modules.contains(&parent),
                "comp '{}' has unknown parent module '{}'",
                instance,
                parent
            );
        }
    }
}

/// The `++ comp` records as (instance, type, parent), in file order.
fn comps_of(recs: &[Rec]) -> Vec<(&str, &str, &str)> {
    recs.iter()
        .filter_map(|r| match r {
            Rec::Comp {
                instance,
                mtype,
                parent,
            } => Some((instance.as_str(), mtype.as_str(), parent.as_str())),
            _ => None,
        })
        .collect()
}

/// The `++ module` record names, in file order.
fn module_names(recs: &[Rec]) -> Vec<&str> {
    recs.iter()
        .filter_map(|r| match r {
            Rec::Module { name, .. } => Some(name.as_str()),
            _ => None,
        })
        .collect()
}

/// Line number (1-based) of the first `module`/`interface`/`program` header
/// named `name` in `src`, mirroring the keyword family xezim's own scan
/// accepts for a storable definition. Independent of that scan: the line's
/// first whitespace token must be one of the header keywords, and the second
/// must be `name` at an identifier boundary (so `top #(...)`, `top;`,
/// `top (input sl)` count, but `top_data` does not).
fn line_of_module(src: &str, name: &str) -> u32 {
    for (n, line) in src.lines().enumerate() {
        let mut tok = line.split_whitespace();
        match tok.next() {
            Some("module") | Some("interface") | Some("program") | Some("macromodule") => {}
            _ => continue,
        }
        if let Some(second) = tok.next() {
            if let Some(after) = second.strip_prefix(name) {
                let boundary = after.is_empty()
                    || !after
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '$');
                if boundary {
                    return (n + 1) as u32;
                }
            }
        }
    }
    panic!("no `module/interface/program/macromodule {name}` header line in source");
}

fn rec_module<'a>(recs: &'a [Rec], name: &str) -> &'a Rec {
    recs.iter()
        .find(|r| matches!(r, Rec::Module { name: n, .. } if n == name))
        .unwrap_or_else(|| panic!("no ++ module record for {}", name))
}

/// Assert the declared line of `name` equals where its header really is.
fn assert_line_exact(recs: &[Rec], src: &str, name: &str) {
    let m = rec_module(recs, name);
    if let Rec::Module { start, .. } = m {
        assert_eq!(*start, line_of_module(src, name), "{} decl line", name);
    } else {
        panic!("{} is a module record", name);
    }
}

#[test]
fn stems_hierarchy_tracks_instances_and_sources() {
    let r = run_case(design(), true);
    // Sanity: the run reached max-time (the normal way these designs end)
    // rather than dying mid-compilation, so the sidecar is this run's, not a
    // leftover.
    assert!(
        r.stderr.contains("reached --max-time"),
        "stderr:\n{}",
        r.stderr
    );
    let recs = parse_stems(&r.stems);
    assert_eq!(
        recs.len(),
        6,
        "3 module records + 3 comp records:\n{}",
        r.stems
    );

    // Module records carry the real source path and the module-header line.
    let top = rec_module(&recs, "top");
    if let Rec::Module {
        file, start, end, ..
    } = top
    {
        assert!(file.contains("a.sv"), "top file is the SV source: {}", file);
        assert_eq!(*start, line_of_module(design(), "top"));
        assert_eq!(*end, *start, "end == start, like xml2stems");
    } else {
        panic!("top is a module record");
    }
    assert_line_exact(&recs, design(), "mid");
    assert_line_exact(&recs, design(), "sub");

    // Comp records link instances to their definitions and parents, both at
    // the top level and one level down (u3 sits under the mid instance).
    let comps = comps_of(&recs);
    assert_eq!(comps.len(), 3, "three instances in the design");
    assert!(comps.contains(&("u1", "sub", "top")));
    assert!(comps.contains(&("u2", "mid", "top")));
    assert!(comps.contains(&("u3", "sub", "mid")));

    // The file is safe for rtlbrowse: one root, every comp resolves.
    validate(&recs);
}

#[test]
fn stems_without_fst_still_writes_the_sidecar() {
    // `--stems` alone (no waveform dump) must still produce the sidecar:
    // the user may add a dump later, and a missing FST is not an error.
    let r = run_case(design(), false);
    let recs = parse_stems(&r.stems);
    assert!(
        recs.iter()
            .any(|rec| matches!(rec, Rec::Module { name, .. } if name == "top"))
    );
    assert_eq!(
        recs.iter()
            .filter(|rec| matches!(rec, Rec::Comp { .. }))
            .count(),
        3
    );
}

#[test]
fn stems_flat_design_has_single_module_no_comps() {
    let flat = "module top;\n  reg a;\n  initial a = 1'b1;\nendmodule\n";
    let r = run_case(flat, true);
    let recs = parse_stems(&r.stems);
    assert_eq!(
        recs.len(),
        1,
        "one module record, no instances:\n{}",
        r.stems
    );
    let top = rec_module(&recs, "top");
    if let Rec::Module { start, .. } = top {
        assert_eq!(*start, line_of_module(flat, "top"));
    } else {
        panic!("top is a module record");
    }
    validate(&recs);
}

#[test]
fn stems_packages_emit_no_module_records() {
    // A package is a definition but not an instantiable module: it must not
    // get a `++ module` record (the writer only names the top plus instance
    // definitions). The single real module stays line-exact.
    let design = "package p;\nendpackage\nmodule top;\n  int q;\n  initial q = 1;\nendmodule\n";
    let r = run_case(design, true);
    let recs = parse_stems(&r.stems); // parse itself asserts numeric lines
    assert_eq!(recs.len(), 1);
    assert_line_exact(&recs, design, "top");
}

#[test]
fn stems_parse_rejects_malformed_input() {
    // Guard the parser replica: a messy line must be refused, not silently
    // accepted — keeping the test honest about the strict `%s` grammar.
    let bad = "++ module top file /a b.sv lines 1 - 1\n";
    let result = std::panic::catch_unwind(|| parse_stems(bad));
    assert!(result.is_err(), "whitespace in a path is a parse error");
}

#[test]
fn stems_repeated_instantiation_emits_one_module_record() {
    // `sub` appears twice as an instance, once as a comp type, and never as a
    // parent: exactly one `++ module sub`. Module records are emitted top
    // first, then each instance's definition in elaboration order; comp
    // records come after all module records, in instance order.
    let r = run_case(design(), true);
    let recs = parse_stems(&r.stems);
    let mods = module_names(&recs);
    assert_eq!(mods, vec!["top", "sub", "mid"], "dedup + first-seen order");
    assert_eq!(recs.len(), 6);

    // Module records first, then comp records — the shape xml2stems emits.
    for (i, rec) in recs.iter().enumerate() {
        if i < 3 {
            assert!(matches!(rec, Rec::Module { .. }), "module block first");
        } else {
            assert!(matches!(rec, Rec::Comp { .. }), "comp block second");
        }
    }
    let comps = comps_of(&recs);
    assert_eq!(
        comps,
        vec![
            ("u1", "sub", "top"),
            ("u2", "mid", "top"),
            ("u3", "sub", "mid")
        ]
    );
    validate(&recs);
}

#[test]
fn stems_multifile_maps_each_module_to_its_own_source() {
    let leaf = "module leaf;\n  reg x;\n  initial x = 0;\nendmodule\n";
    let top = "module top;\n  leaf u1 ();\nendmodule\n";
    // b.sv passed before a.sv deliberately, so file index 0 is NOT the file
    // that holds the top module — the per-module mapping must still be right.
    let r = run_case_multi(&[("b", leaf), ("a", top)], true);
    let recs = parse_stems(&r.stems);
    assert_eq!(recs.len(), 3);
    for name in ["top", "leaf"] {
        let m = rec_module(&recs, name);
        if let Rec::Module { file, start, .. } = m {
            let want = if name == "top" { "a.sv" } else { "b.sv" };
            assert!(
                file.contains(want) && !file.contains(if want == "a.sv" { "b.sv" } else { "a.sv" }),
                "{} maps to its own file: {}",
                name,
                file
            );
            let src = if name == "top" { top } else { leaf };
            assert_eq!(*start, line_of_module(src, name));
        } else {
            panic!("{} is a module record", name);
        }
    }
    assert!(!r.stems.contains("<unknown>"), "both files resolve");
    validate(&recs);
}

#[test]
fn stems_deep_chain_resolves_intermediate_parents() {
    // top → b1 → b2 → leaf: each comp's parent names the CONTAINING module's
    // definition, even when that module is itself only an instance (b1, b2).
    let sv = "\
module leaf;
  reg y;
  initial y = 0;
endmodule

module b2;
  leaf u3 ();
endmodule

module b1;
  b2 u2 ();
endmodule

module top;
  b1 u1 ();
endmodule
";
    let r = run_case(sv, true);
    let recs = parse_stems(&r.stems);
    assert_eq!(recs.len(), 7, "4 module records + 3 comps:\n{}", r.stems);
    for name in ["top", "b1", "b2", "leaf"] {
        assert_line_exact(&recs, sv, name);
    }
    let comps = comps_of(&recs);
    assert!(comps.contains(&("u1", "b1", "top")));
    assert!(comps.contains(&("u2", "b2", "b1")));
    assert!(comps.contains(&("u3", "leaf", "b2")));
    validate(&recs);
}

#[test]
fn stems_ansi_and_param_headers_find_their_line() {
    let sv = "\
module h1 (input clk);
  reg q;
  initial q = clk;
endmodule

module h2 #(parameter W = 8);
endmodule

module top;
  h1 u1 (.clk(1'b0));
  h2 u2 ();
endmodule
";
    let r = run_case(sv, true);
    let recs = parse_stems(&r.stems);
    assert_eq!(recs.len(), 5, "3 modules + 2 comps:\n{}", r.stems);
    // `module h1 (input clk);` and `module h2 #(parameter W = 8);` are ANSI /
    // parameterized headers; the declaration scan must still name their real
    // lines rather than falling back to 1.
    for name in ["top", "h1", "h2"] {
        assert_line_exact(&recs, sv, name);
    }
    validate(&recs);
}

#[test]
fn stems_colliding_instance_names_keep_parent_linkage() {
    // Instance `u` exists both at the top and under an instance of `mid`: the
    // two `++ comp u` records differ only by parent, and both parents must
    // resolve to a module record.
    let sv = "\
module leaf;
  reg x;
  initial x = 0;
endmodule

module mid;
  leaf u ();
endmodule

module top;
  leaf u ();
  mid w ();
endmodule
";
    let r = run_case(sv, true);
    let recs = parse_stems(&r.stems);
    let comps = comps_of(&recs);
    assert_eq!(comps.len(), 3);
    assert!(comps.contains(&("u", "leaf", "top")));
    assert!(comps.contains(&("u", "leaf", "mid")));
    assert!(comps.contains(&("w", "mid", "top")));
    validate(&recs);
}

#[test]
fn stems_interface_instance_resolves_like_a_module() {
    // An interface instantiates like a module: rtlbrowse links the comp to a
    // `++ module` record named after the interface (the way Verilator's
    // xml2stems treats interface comps), so the tree has no `[MISSING]` node.
    let sv = "\
interface bus;
  logic a;
endinterface

module top;
  bus u1 ();
endmodule
";
    let r = run_case(sv, true);
    let recs = parse_stems(&r.stems);
    assert_eq!(
        recs.len(),
        3,
        "bus + top module records, one comp:\n{}",
        r.stems
    );
    assert!(module_names(&recs).contains(&"bus"));
    assert_line_exact(&recs, sv, "bus");
    assert_line_exact(&recs, sv, "top");
    validate(&recs);
}

#[test]
fn stems_forward_reference_keeps_the_decl_line() {
    // `sub` is declared AFTER the module that instantiates it; its record
    // still carries sub's real header line, not the top's.
    let sv = "\
module top;
  sub u1 ();
endmodule

module sub;
  reg s;
  initial s = 0;
endmodule
";
    let r = run_case(sv, true);
    let recs = parse_stems(&r.stems);
    assert_eq!(recs.len(), 3);
    let (ts, ss) = {
        let top = rec_module(&recs, "top");
        let sub = rec_module(&recs, "sub");
        if let (Rec::Module { start: ts, .. }, Rec::Module { start: ss, .. }) = (top, sub) {
            (*ts, *ss)
        } else {
            panic!("both are module records");
        }
    };
    assert_eq!(ts, line_of_module(sv, "top"));
    assert_eq!(ss, line_of_module(sv, "sub"));
    assert!(ss > ts, "sub is declared after top: sub={}, top={}", ss, ts);
    validate(&recs);
}

#[test]
fn stems_comments_and_attributes_do_not_move_the_decl_line() {
    // Comment text that reads like a module header, and an attribute on its
    // own line, must neither create phantom records nor shift a real module's
    // declared line. The preprocessor blanks comments and inline attributes
    // to spaces while preserving line counts, keeping preprocessed line
    // numbers equal to the raw source's.
    let sv = "\
// module fake_thing;
/*
 * module also_fake;
 */
(* dont_touch *)
module top;
  // not a scope boundary: module deeper_thing
endmodule
";
    let r = run_case(sv, true);
    let recs = parse_stems(&r.stems);
    assert_eq!(
        recs.len(),
        1,
        "one real module; comments forge nothing:\n{}",
        r.stems
    );
    assert_eq!(module_names(&recs), vec!["top"]);
    let m = rec_module(&recs, "top");
    if let Rec::Module { start, file, .. } = m {
        assert!(!file.contains("<unknown>"));
        assert_eq!(*start, line_of_module(sv, "top"));
    } else {
        panic!("top is a module record");
    }
}

#[test]
fn stems_escaped_header_falls_back_to_line1() {
    // An escaped-identifier header (`module \top ;`) is something the
    // first-token scan does not recognize (the token is `\top`), so the lines
    // pair falls back to the documented 1-1 and the sidecar still names the
    // real file. This is the honest "window opens at the top of the file"
    // degradation, plus proof a fallback never writes a non-numeric line
    // (parse_stems asserts the shape).
    let sv = "module \\top ;\n  int q;\n  initial q = 1;\nendmodule\n";
    let r = run_case(sv, true);
    let recs = parse_stems(&r.stems);
    assert_eq!(recs.len(), 1);
    let m = rec_module(&recs, "top");
    if let Rec::Module {
        file, start, end, ..
    } = m
    {
        assert!(file.contains("a.sv"));
        assert_eq!(*start, 1, "unrecognized header -> line 1");
        assert_eq!(*end, 1);
    } else {
        panic!("top is a module record");
    }
}

#[test]
fn stems_artifact_load_keeps_paths_falls_back_to_line1() {
    // A compiled-artifact run retains the serialized instance tree and
    // module→file map but no preprocessed text (`source_texts` is not
    // serialized), so line pairs land at the documented 1-1 while file paths
    // stay real — the sidecar keeps working when the design is shipped as an
    // artifact instead of sources.
    let dir = new_scratch("art");
    let sv = design();
    let sv_path = dir.join("a.sv");
    std::fs::write(&sv_path, sv).expect("write sv");
    let art = dir.join("a.xez");

    let (ok, stderr) = invoke_xezim(
        &dir,
        &[
            "-o".to_string(),
            art.display().to_string(),
            "--compile".to_string(),
            "-s".to_string(),
            "top".to_string(),
            sv_path.display().to_string(),
        ],
    );
    assert!(ok, "artifact compile:\n{stderr}");
    assert!(art.exists(), "artifact written");

    let stems = dir.join("a.stems");
    let (ok, stderr) = invoke_xezim(
        &dir,
        &[
            "--simulate".to_string(),
            "--max-time".to_string(),
            "100".to_string(),
            "-s".to_string(),
            "top".to_string(),
            "--stems".to_string(),
            stems.display().to_string(),
            art.display().to_string(),
        ],
    );
    assert!(ok, "artifact load:\n{stderr}");
    let recs = parse_stems(&std::fs::read_to_string(&stems).expect("sidecar"));
    assert_eq!(recs.len(), 6, "artifact keeps the full instance tree");
    for name in ["top", "sub", "mid"] {
        let m = rec_module(&recs, name);
        if let Rec::Module {
            file, start, end, ..
        } = m
        {
            assert!(file.contains("a.sv"), "{} keeps its real path", name);
            assert_eq!(*start, 1, "{} falls back, no text in the artifact", name);
            assert_eq!(*end, 1);
        } else {
            panic!("{} is a module record", name);
        }
    }
    // Tree is still usable: one root, every comp resolves.
    validate(&recs);
}

#[test]
fn stems_elaboration_failure_writes_no_sidecar() {
    // An undefined module aborts elaboration before the simulator exists, so
    // `--stems` must not leave a file behind (a stale sidecar would silently
    // pair the wrong source with whatever dump was produced).
    let bad = "module top;\n  nope u ();\nendmodule\n";
    let (stderr, exists) = run_case_failure(bad);
    assert!(!exists, "no sidecar on elaboration failure");
    assert!(
        stderr.contains("nope"),
        "run names the culprit:\n{}",
        stderr
    );
}

#[test]
fn stems_whitespace_in_source_path_breaks_the_grammar() {
    // The `%s` grammar rtlbrowse uses cannot represent a path containing a
    // space. xezim passes the path through verbatim (the documented, xml2stems-
    // matching limit), so the emitted module record still shows the real file,
    // and this test pins that the tokenizing parser — standing in for
    // `load_stems_file` — rejects it rather than misinterpreting the path.
    let sv = "module top;\nendmodule\n";
    let dir = std::env::temp_dir().join(format!(
        "xezim stems space_{}_{}",
        std::process::id(),
        DIR_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let sv_path = dir.join("a.sv");
    std::fs::write(&sv_path, sv).expect("write sv");
    let stems = dir.join("a.stems");
    let args: Vec<String> = vec![
        "--simulate".to_string(),
        "--max-time".to_string(),
        "100".to_string(),
        "-s".to_string(),
        "top".to_string(),
        "--stems".to_string(),
        stems.display().to_string(),
        sv_path.display().to_string(),
    ];
    let (ok, _stderr) = invoke_xezim(&dir, &args);
    assert!(ok);
    let raw = std::fs::read_to_string(&stems).expect("sidecar");
    let line = raw
        .lines()
        .find(|l| l.starts_with("++ module top "))
        .expect("module record");
    assert!(
        line.contains("stems space_"),
        "path emitted verbatim: {line}"
    );
    assert!(!line.contains("<unknown>"));
    assert!(
        std::panic::catch_unwind(|| parse_stems(&raw)).is_err(),
        "a space in the path must read as a grammar error"
    );
}

/// A `macromodule` header (§3.12) resolves to its real declaration line, not
/// the line-1 fallback: the keyword family is module/interface/program plus
/// macromodule.
#[test]
fn stems_macromodule_header_finds_its_line() {
    let sv = "\
// fixture comments push the header off line 1
// and a second one
macromodule sink (output logic q);
  assign q = 1'b1;
endmodule

module top;
  logic q;
  sink u_sink (.q(q));
endmodule
";
    let r = run_case(sv, false);
    let recs = parse_stems(&r.stems);
    assert_eq!(recs.len(), 3, "sink + top module records, one comp");
    assert_eq!(line_of_module(sv, "sink"), 3, "the oracle agrees: line 3");
    assert_line_exact(&recs, sv, "sink");
    assert_line_exact(&recs, sv, "top");
    validate(&recs);
}

/// A module defined inside an `\`include`d file is attributed to the
/// including file at the spliced position: xezim-core keys module provenance
/// per top-level source and keeps no include map. The output stays
/// structurally valid, but the file/line pair points into the includer —
/// pins today's behavior so a future core change that records per-included-
/// file spans shows up here.
#[test]
fn stems_include_spliced_modules_report_the_including_file() {
    let dir = new_scratch("include");
    std::fs::write(
        dir.join("defs.svh"),
        "// leaf header comment\n// and another\nmodule leaf (output logic q);\n  assign q = 1'b1;\nendmodule\n",
    )
    .expect("write defs.svh");
    let top_sv = "\
`include \"defs.svh\"
module top;
  logic q;
  leaf u_leaf (.q(q));
endmodule
";
    let sv_path = dir.join("top.sv");
    std::fs::write(&sv_path, top_sv).expect("write top.sv");
    let stems = dir.join("t.stems");
    let args: Vec<String> = vec![
        "--simulate".to_string(),
        "--max-time".to_string(),
        "100".to_string(),
        "-s".to_string(),
        "top".to_string(),
        "--stems".to_string(),
        stems.display().to_string(),
        sv_path.display().to_string(),
    ];
    let (ok, stderr) = invoke_xezim(&dir, &args);
    assert!(ok, "xezim run failed:\n{stderr}");
    let raw = std::fs::read_to_string(&stems).expect("sidecar");
    let recs = parse_stems(&raw);
    assert_eq!(recs.len(), 3, "top + spliced leaf module records, one comp");
    // defs.svh is 5 lines and its third line is the `module leaf` header, so
    // the splice puts it at expanded line 3 — attributed to top.sv, not
    // defs.svh. top's own header lands on the expanded 6th line, not its raw
    // line 3, because the include above it adds lines.
    if let Rec::Module {
        file, start, end, ..
    } = rec_module(&recs, "leaf")
    {
        assert!(file.ends_with("top.sv"), "includer is attributed: {file}");
        assert_eq!(*start, 3, "spliced position, not defs.svh's real line");
        assert_eq!(*end, *start);
    } else {
        panic!("leaf is a module record");
    }
    if let Rec::Module { start, .. } = rec_module(&recs, "top") {
        assert_eq!(*start, 6, "include shifts the includer's own header too");
    } else {
        panic!("top is a module record");
    }
    validate(&recs);
}

/// A stems path that cannot be created warns to stderr and the run exits 0 —
/// a bad sidecar must never take the simulation down with it.
#[test]
fn stems_unwritable_path_warns_and_continues() {
    let dir = new_scratch("badstems");
    let sv_path = dir.join("a.sv");
    std::fs::write(&sv_path, "module top;\nendmodule\n").expect("write sv");
    let stems = dir.join("no/such/dir/x.stems");
    let args: Vec<String> = vec![
        "--simulate".to_string(),
        "--max-time".to_string(),
        "100".to_string(),
        "-s".to_string(),
        "top".to_string(),
        "--stems".to_string(),
        stems.display().to_string(),
        sv_path.display().to_string(),
    ];
    let (ok, stderr) = invoke_xezim(&dir, &args);
    assert!(ok, "a bad stems path must not fail the run:\n{stderr}");
    assert!(
        stderr.contains("cannot write stems file"),
        "the warning names the failure: {stderr}"
    );
    assert!(!stems.exists(), "no sidecar was written");
}

/// `--stems=<file>` parses identically to `--stems <file>`.
#[test]
fn stems_equals_form_parses_the_same() {
    let dir = new_scratch("equals");
    let sv_path = dir.join("a.sv");
    std::fs::write(&sv_path, "module top;\nendmodule\n").expect("write sv");
    let stems = dir.join("eq.stems");
    let _ = std::fs::remove_file(&stems);
    let args: Vec<String> = vec![
        "--simulate".to_string(),
        "--max-time".to_string(),
        "100".to_string(),
        "-s".to_string(),
        "top".to_string(),
        format!("--stems={}", stems.display()),
        sv_path.display().to_string(),
    ];
    let (ok, stderr) = invoke_xezim(&dir, &args);
    assert!(ok, "xezim run failed:\n{stderr}");
    assert!(stems.exists(), "the `=` form writes the sidecar");
    let recs = parse_stems(&std::fs::read_to_string(&stems).expect("sidecar"));
    assert_eq!(recs.len(), 1, "flat design: a single module record");
    validate(&recs);
}
