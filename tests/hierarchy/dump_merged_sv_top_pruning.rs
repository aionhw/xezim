//! `--dump-merged-sv` with `-s <top>`: keep only the files needed to elaborate
//! that top.
//!
//! The flag exists to turn a large shared `-f` build into a re-runnable repro,
//! and in a shared file list most files belong to some *other* top — so without
//! pruning the "minimal example" is the whole build. The closure is lexical (it
//! runs before parsing, so the dump still works when the design does not
//! elaborate, which is the situation the flag is for) and conservative: it may
//! keep a file more than strictly needed, never one fewer.
//!
//! What must hold: the selection is correct for the obvious cases, and — the
//! property that actually matters — whatever comes out still re-runs on its
//! own.

use std::process::Command;

fn xezim_bin() -> std::path::PathBuf {
    // Cargo names the binary it built for the integration tests.
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_xezim"))
}

/// Writes `files` into a fresh directory and returns their paths in order.
fn scratch(tag: &str, files: &[(&str, &str)]) -> (std::path::PathBuf, Vec<std::path::PathBuf>) {
    let dir = std::env::temp_dir().join(format!("xezim_prune_{}_{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    let mut paths = Vec::new();
    for (name, body) in files {
        let p = dir.join(name);
        std::fs::write(&p, body).expect("write");
        paths.push(p);
    }
    (dir, paths)
}

/// Basenames of the `// ===== file n/m: <path> =====` sections, in order.
fn sections(merged: &std::path::Path) -> Vec<String> {
    let text = std::fs::read_to_string(merged).expect("read merged");
    text.lines()
        .filter_map(|l| l.strip_prefix("// ===== file "))
        .filter_map(|l| l.split_once(": "))
        .map(|(_, rest)| {
            rest.trim_end_matches(" =====")
                .rsplit('/')
                .next()
                .unwrap_or("")
                .to_string()
        })
        .collect()
}

const DESIGN: &[(&str, &str)] = &[
    (
        "pkg_a.sv",
        "package pkg_a;\n  typedef struct packed { logic [7:0] d; logic v; } beat_t;\nendpackage\n",
    ),
    (
        "pkg_unused.sv",
        "package pkg_unused;\n  parameter int NOPE = 1;\nendpackage\n",
    ),
    (
        "if_bus.sv",
        "interface bus_if;\n  logic clk;\n  modport mp (input clk);\nendinterface\n",
    ),
    (
        "leaf.sv",
        "module leaf import pkg_a::*; (input logic clk, output beat_t o);\n\
         always_ff @(posedge clk) o.d <= o.d + 1'b1;\nendmodule\n",
    ),
    (
        "mid.sv",
        "module mid import pkg_a::*; (input logic clk, output beat_t o);\n\
         leaf u_leaf (.clk(clk), .o(o));\nendmodule\n",
    ),
    (
        "dut_top.sv",
        "module dut_top import pkg_a::*; (bus_if.mp b, output beat_t o);\n\
         mid u_mid (.clk(b.clk), .o(o));\nendmodule\n",
    ),
    (
        "other_top.sv",
        "module other_top;\n  unrelated u_x ();\nendmodule\n",
    ),
    (
        "unrelated.sv",
        "module unrelated;\n  import pkg_unused::*;\n  initial $display(\"nope %0d\", NOPE);\nendmodule\n",
    ),
    (
        "tb.sv",
        "module tb;\n  import pkg_a::*;\n  bus_if b();\n  beat_t o;\n\
         dut_top u_dut (.b(b.mp), .o(o));\n\
         initial begin b.clk = 0; #10 $finish; end\n\
         always #5 b.clk = ~b.clk;\nendmodule\n",
    ),
];

fn dump(tag: &str, files: &[(&str, &str)], top: Option<&str>) -> (std::path::PathBuf, String) {
    let bin = xezim_bin();
    let (dir, paths) = scratch(tag, files);
    let merged = dir.join("merged.sv");
    let mut cmd = Command::new(&bin);
    cmd.arg("--parse");
    if let Some(t) = top {
        cmd.args(["-s", t]);
    }
    cmd.arg("--dump-merged-sv").arg(&merged);
    for p in &paths {
        cmd.arg(p);
    }
    let out = cmd.output().expect("run xezim");
    let log = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (merged, log)
}

/// `-s` selects the transitive closure and drops everything else.
#[test]
fn top_selects_only_reachable_files() {
    if !xezim_bin().exists() {
        return;
    }
    let (merged, _log) = dump("tb", DESIGN, Some("tb"));
    let mut got = sections(&merged);
    got.sort();
    assert_eq!(
        got,
        vec![
            "dut_top.sv",
            "if_bus.sv",
            "leaf.sv",
            "mid.sv",
            "pkg_a.sv",
            "tb.sv"
        ],
        "closure from tb"
    );
}

/// A different top in the SAME file list selects a disjoint set — the case that
/// makes the flag useful on a shared `-f`.
#[test]
fn a_different_top_selects_its_own_files() {
    if !xezim_bin().exists() {
        return;
    }
    let (merged, _log) = dump("other", DESIGN, Some("other_top"));
    let mut got = sections(&merged);
    got.sort();
    assert_eq!(got, vec!["other_top.sv", "pkg_unused.sv", "unrelated.sv"]);
}

/// Without `-s`, every input is dumped — the pre-existing behavior.
#[test]
fn no_top_dumps_everything() {
    if !xezim_bin().exists() {
        return;
    }
    let (merged, _log) = dump("all", DESIGN, None);
    assert_eq!(sections(&merged).len(), DESIGN.len());
}

/// The pruned dump must still elaborate and run on its own — the property the
/// whole flag exists for.
#[test]
fn pruned_dump_reruns_standalone() {
    let bin = xezim_bin();
    if !bin.exists() {
        return;
    }
    let (merged, _log) = dump("rerun", DESIGN, Some("tb"));
    let out = Command::new(&bin)
        .args(["--simulate", "-s", "tb", "--max-time", "100ns"])
        .arg(&merged)
        .output()
        .expect("rerun");
    let log = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        log.contains("$finish called"),
        "pruned dump did not run: {}",
        log
    );
}

/// Classes in packages, an `interface class` (whose terminator is `endclass`,
/// not `endinterface` — mis-tracking it unbalances every declaration after it
/// in the file) and a `typedef class` forward declaration.
#[test]
fn class_packages_and_interface_class() {
    if !xezim_bin().exists() {
        return;
    }
    let files: &[(&str, &str)] = &[
        (
            "base_pkg.sv",
            "package base_pkg;\n  virtual class base_obj;\n\
             pure virtual function string kind();\n  endclass\n\
             class simple_obj extends base_obj;\n\
             function string kind(); return \"simple\"; endfunction\n  endclass\n\
             typedef class fwd_only;\nendpackage\n",
        ),
        (
            "agent_pkg.sv",
            "package agent_pkg;\n  import base_pkg::*;\n\
             class driver extends simple_obj;\n    int sent;\n\
             task run(); sent++; endtask\n  endclass\nendpackage\n",
        ),
        (
            "sb_pkg.sv",
            "package sb_pkg;\n  import base_pkg::*;\n\
             class sb extends base_obj;\n\
             function string kind(); return \"sb\"; endfunction\n  endclass\nendpackage\n",
        ),
        (
            "iface.sv",
            "interface class ifc_class;\n  pure virtual function int idv();\nendclass\n\
             interface dut_if;\n  logic clk;\nendinterface\n",
        ),
        (
            "dut.sv",
            "module dut (dut_if vif);\n  always_ff @(posedge vif.clk) ;\nendmodule\n",
        ),
        (
            "tb_uvm.sv",
            "module tb_uvm;\n  import agent_pkg::*;\n  dut_if vif();\n\
             dut u_dut (.vif(vif));\n  driver d;\n\
             initial begin d = new(); d.run();\n\
             $display(\"KIND %s SENT %0d\", d.kind(), d.sent); #10 $finish; end\n\
             initial vif.clk = 0;\n  always #5 vif.clk = ~vif.clk;\nendmodule\n",
        ),
    ];
    let (merged, _log) = dump("uvm", files, Some("tb_uvm"));
    let got = sections(&merged);
    // `dut_if` is declared AFTER the interface class in iface.sv, so it is only
    // found at top level if the interface-class terminator was tracked right.
    assert!(
        got.contains(&"iface.sv".to_string()),
        "iface.sv missing: {:?}",
        got
    );
    assert!(
        got.contains(&"base_pkg.sv".to_string()),
        "base pkg missing: {:?}",
        got
    );
    assert!(
        got.contains(&"agent_pkg.sv".to_string()),
        "agent pkg missing: {:?}",
        got
    );
    assert!(
        !got.contains(&"sb_pkg.sv".to_string()),
        "unused pkg kept: {:?}",
        got
    );

    let out = Command::new(xezim_bin())
        .args(["--simulate", "-s", "tb_uvm", "--max-time", "100ns"])
        .arg(&merged)
        .output()
        .expect("rerun");
    let log = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        log.contains("KIND simple SENT 1"),
        "pruned UVM-style dump: {}",
        log
    );
}

/// A top that no input file declares (it may come from a `-v`/`-y` library):
/// warn and fall back to dumping everything rather than emitting a file that
/// is missing the design.
#[test]
fn unknown_top_falls_back_to_full_dump() {
    if !xezim_bin().exists() {
        return;
    }
    let (merged, log) = dump("unknown", DESIGN, Some("no_such_module"));
    assert_eq!(sections(&merged).len(), DESIGN.len(), "should dump all");
    assert!(
        log.contains("not declared by any input file"),
        "no warning: {}",
        log
    );
}

/// A name that only appears inside a comment or a string literal must not pull
/// its file in — the scan is a lexer pass, not a text search.
#[test]
fn comments_and_strings_do_not_create_dependencies() {
    if !xezim_bin().exists() {
        return;
    }
    let files: &[(&str, &str)] = &[
        (
            "heavy.sv",
            "module heavy;\n  initial $display(\"heavy\");\nendmodule\n",
        ),
        (
            "small_tb.sv",
            "module small_tb;\n  // instantiate heavy here one day\n\
             initial begin $display(\"not heavy\"); #1 $finish; end\nendmodule\n",
        ),
    ];
    let (merged, _log) = dump("comments", files, Some("small_tb"));
    assert_eq!(
        sections(&merged),
        vec!["small_tb.sv"],
        "comment pulled a file in"
    );
}

/// A file that declares NO design unit is never referenced by name, so a pure
/// reachability walk drops it — but it may carry §3.12 compilation-unit
/// declarations (a file-scope `typedef`/function) or a top-level `bind`, which
/// the rest of the design uses without ever naming the file.
///
/// This is the failure mode that matters most: dropping them does not simply
/// fail to compile, it can leave a dump that still runs and reports a
/// DIFFERENT answer. Here the $unit `typedef`/function are needed for the
/// design to behave at all, and the bind pulls in a checker module nothing
/// else mentions.
#[test]
fn unit_scope_and_bind_files_are_never_dropped() {
    let bin = xezim_bin();
    if !bin.exists() {
        return;
    }
    let files: &[(&str, &str)] = &[
        (
            "unit_scope.sv",
            "typedef logic [15:0] word_t;\n\
             function automatic word_t dbl(input word_t a); return a << 1; endfunction\n",
        ),
        (
            "checker_mod.sv",
            "module chk (input logic clk, input word_t v);\n\
             always @(posedge clk) if (v === 16'hFFFF) $display(\"CHK\");\nendmodule\n",
        ),
        (
            "bindfile.sv",
            "bind dut_a chk u_chk (.clk(clk), .v(val));\n",
        ),
        (
            "dut_a.sv",
            "module dut_a (input logic clk, output word_t val);\n\
             always_ff @(posedge clk) val <= dbl(val) + 16'd1;\nendmodule\n",
        ),
        (
            "tb2.sv",
            "module tb2;\n  logic clk = 0;\n  word_t val;\n\
             dut_a u (.clk(clk), .val(val));\n  always #5 clk = ~clk;\n\
             initial begin #40; $display(\"VAL %0d\", val); $finish; end\nendmodule\n",
        ),
    ];
    let (merged, _log) = dump("unitscope", files, Some("tb2"));
    let got = sections(&merged);
    for needed in ["unit_scope.sv", "bindfile.sv", "tb2.sv", "dut_a.sv"] {
        assert!(
            got.contains(&needed.to_string()),
            "{} dropped: {:?}",
            needed,
            got
        );
    }
    // Reachable ONLY through the bind directive.
    assert!(
        got.contains(&"checker_mod.sv".to_string()),
        "bind target dropped: {:?}",
        got
    );

    // The decisive check: same answer as the unpruned build, not merely "it
    // compiles". Without the $unit file this printed VAL 1 instead of VAL x.
    let out = Command::new(&bin)
        .args(["--simulate", "-s", "tb2", "--max-time", "100ns"])
        .arg(&merged)
        .output()
        .expect("rerun");
    let log = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        log.contains("VAL x"),
        "pruned dump changed the answer: {}",
        log
    );
}
