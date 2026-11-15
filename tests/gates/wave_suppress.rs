//! `--wave-suppress` must cut a subtree out of every waveform sink.
//!
//! The exclusion list mixes two kinds of entry, and the two are resolved in
//! completely different places: a bare identifier is a MODULE name and must
//! reach every instance of that definition wherever it sits (here `probe`
//! appears both directly under `top` and one level deeper, inside `holder`),
//! while a dotted identifier is a hierarchical PATH and must reach exactly the
//! entries beneath it â `top.u_h2` takes a whole instance, `u_h3.h` takes one
//! net of a sibling that must otherwise survive intact, and `u_h1` has to come
//! through both unscathed.
//! A single wrong `starts_with` either leaves a bank of memory in the dump or
//! quietly deletes its untouched siblings, and both look fine until someone
//! opens the file, so the checks are on the names the writer actually emitted.
//!
//! The same list is shared by VCD, FST and XTrace, and an explicit include
//! scope still outranks it: `--fst-scope top.u_h2` brings that subtree back.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::BufReader;
use std::process::Command;

use fst_reader::{FstHierarchyEntry, FstReader};

const SRC: &str = r#"
`timescale 1ns/1ps
module probe(input logic clk, input logic [7:0] d, output logic [7:0] q);
  logic [7:0] s;
  logic       v;
  assign q = s;
  always @(posedge clk) begin
    s <= d + 8'd1;
    v <= ^d;
  end
endmodule
module holder(input logic clk, input logic [7:0] d, output logic [7:0] q);
  logic [7:0] h;
  logic [7:0] pw;
  probe u_probe(.clk(clk), .d(d), .q(pw));
  always @(posedge clk) h <= d + 8'd2;
  assign q = h ^ pw;
endmodule
module plain(input logic clk, input logic [7:0] d, output logic [7:0] q);
  logic [7:0] p;
  always @(posedge clk) p <= d + 8'd3;
  assign q = p;
endmodule
module bank(input logic clk, input logic [7:0] d, output logic [7:0] q);
  logic [7:0] m;
  logic       g;
  always @(posedge clk) begin
    m <= d + 8'd4;
    g <= ^d;
  end
  assign q = m;
endmodule
module top;
  logic clk = 1'b0;
  logic [7:0] din = 8'd0;
  logic [7:0] qa, qb, qc, qd, qe;
  always #5 clk = ~clk;
  probe  u_p1(.clk(clk), .d(din), .q(qa));
  holder u_h1(.clk(clk), .d(din), .q(qb));
  holder u_h2(.clk(clk), .d(din), .q(qc));
  holder u_h3(.clk(clk), .d(din), .q(qd));
  plain  u_pl(.clk(clk), .d(din), .q(qe));
  // Three generated lanes, each holding one `bank`. A generated instance is
  // named by its INDEX (`g_lane[1].u_bank`), the spelling the signal table
  // carries and that an exclusion entry has to match (#1103).
  genvar gi;
  generate
    for (gi = 0; gi < 3; gi++) begin : g_lane
      logic [7:0] qw;
      bank u_bank(.clk(clk), .d(din), .q(qw));
    end
  endgenerate
  initial begin
    $dumpfile("@VCD@");
    $dumpvars(0, top);
    #60;
    $finish;
  end
endmodule
"#;

/// The top module, as `-s` names it and as every dumped path is prefixed.
const TOP: &str = "top";

/// Every entry the exclusion list must remove, by both kinds of entry.
const ABSENT: &[&str] = &[
    // module name `probe`: the instance sitting directly under top ...
    "top.u_p1.s",
    "top.u_p1.v",
    // ... and the ones buried inside a surviving instance.
    "top.u_h1.u_probe.s",
    "top.u_h1.u_probe.v",
    // path entry with the top prefix: the whole subtree.
    "top.u_h2.h",
    "top.u_h2.pw",
    "top.u_h2.u_probe.s",
    // path entry without it, naming ONE net: the rest of that instance stays.
    "top.u_h3.h",
];

/// What must survive them: the sibling holders â all of `u_h1`, and all of
/// `u_h3` except the one net â the plain instance, and top's own nets,
/// including `qa`, which an excluded instance drives.
const PRESENT: &[&str] = &[
    "top.clk",
    "top.din",
    "top.qa",
    "top.qb",
    "top.qc",
    "top.qd",
    "top.u_h1.h",
    "top.u_h1.pw",
    "top.u_h3.pw",
    "top.u_pl.p",
    "top.u_pl.q",
];

/// The list under test: one module name, one path with the top prefix, one
/// path without it.
const SUPPRESS: &[&str] = &["probe", "top.u_h2", "u_h3.h"];

/// Fully-qualified names a VCD declares, rebuilt from the `$scope` stack â
/// the writer nests the tree, it does not print dotted names flat.
fn vcd_names(path: &std::path::Path) -> HashSet<String> {
    let text = fs::read_to_string(path).expect("read vcd");
    let mut out = HashSet::new();
    let mut stack: Vec<String> = Vec::new();
    for line in text.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        match parts.first().copied() {
            Some("$scope") if parts.len() >= 4 => stack.push(parts[2..parts.len() - 1].join(".")),
            Some("$upscope") => {
                stack.pop();
            }
            Some("$var") if parts.len() >= 6 => {
                // `$var <kind> <size> <code> <name> [range] $end`; the range is
                // not part of the name the exclusion list is matched against.
                let mut field = &parts[4..parts.len() - 1];
                if field.last().is_some_and(|t| t.starts_with('[')) {
                    field = &field[..field.len() - 1];
                }
                out.insert(
                    stack
                        .iter()
                        .map(|s| s.as_str())
                        .chain(field.iter().copied())
                        .collect::<Vec<_>>()
                        .join("."),
                );
            }
            Some("$enddefinitions") => break,
            _ => {}
        }
    }
    out
}

/// The same set, decoded from an FST.
fn fst_names(path: &std::path::Path) -> HashSet<String> {
    let file = fs::File::open(path).expect("open fst");
    let mut reader = FstReader::open_and_read_time_table(BufReader::new(file))
        .expect("FST header does not decode");
    let mut out = HashSet::new();
    let mut stack: Vec<String> = Vec::new();
    reader
        .read_hierarchy(|e| {
            match e {
                FstHierarchyEntry::Scope { name, .. } => stack.push(name.clone()),
                FstHierarchyEntry::UpScope => {
                    stack.pop();
                }
                FstHierarchyEntry::Var { name, .. } => {
                    // FST carries a bit range inside the var name (`s [7:0]`).
                    let name = match name.split_once(" [") {
                        Some((n, _)) => n.to_string(),
                        None => name.clone(),
                    };
                    let mut full: Vec<&str> = stack.iter().map(|s| s.as_str()).collect();
                    full.push(&name);
                    out.insert(full.join("."));
                }
                _ => {}
            }
        })
        .expect("FST hierarchy does not decode");
    out
}

/// The same set, decoded from an XTrace: `M,<id>,<path>` records name the
/// scopes, `S,<sid>,<id>,<leaf>,<type>,<attrs>` the leaves beneath them.
fn xtrace_names(path: &std::path::Path) -> HashSet<String> {
    let text = fs::read_to_string(path).expect("read xtrace");
    let mut out = HashSet::new();
    let mut scopes: HashMap<String, String> = HashMap::new();
    for line in text.lines() {
        if let Some(r) = line.strip_prefix("M,") {
            if let [id, p] = r.splitn(2, ',').collect::<Vec<_>>()[..] {
                // Scope paths are `/top/u_h1`; the names compared here are the
                // dotted ones the other two sinks emit.
                scopes.insert(id.to_string(), p.trim_start_matches('/').replace('/', "."));
            }
        } else if let Some(r) = line.strip_prefix("S,") {
            if let [_, id, leaf, _] = r.splitn(4, ',').collect::<Vec<_>>()[..] {
                let base = scopes.get(id).cloned().unwrap_or_else(|| TOP.to_string());
                out.insert(format!("{}.{}", base, leaf));
            }
        }
    }
    out
}

/// Elaborate, simulate and dump `SRC` to all three sinks; returns xezim's
/// stderr for the run.
fn dump(
    sv: &std::path::Path,
    vcd: &std::path::Path,
    fst: &std::path::Path,
    xt: &std::path::Path,
    extra: &[&str],
) -> String {
    fs::write(sv, SRC.replace("@VCD@", &vcd.to_string_lossy())).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_xezim"));
    cmd.arg("--no-cache")
        .arg("--wave")
        .arg("--xtrace")
        .arg(xt)
        .arg("-s")
        .arg(TOP)
        .arg(sv);
    for e in extra {
        cmd.arg(e);
    }
    // `--fst <path>` right after the format switch.
    if extra.iter().any(|a| *a == "--fst") {
        cmd.arg(fst);
    }
    let out = cmd.output().expect("run xezim");
    assert!(out.status.success(), "xezim failed: {:?}", out);
    String::from_utf8_lossy(&out.stderr).to_string()
}

fn tmp(stem: &str, ext: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "xezim_wave_suppress_{}_{}.{}",
        stem,
        std::process::id(),
        ext
    ));
    let _ = fs::remove_file(&p);
    p
}

/// The list as command-line arguments: one repeated option per entry.
fn suppress_args<'a>(entries: &'a [&'a str]) -> Vec<&'a str> {
    let mut v = Vec::new();
    for e in entries {
        v.push("--wave-suppress");
        v.push(*e);
    }
    v
}

/// One module name and two paths â one written with its top-module prefix, one
/// without â must remove exactly what they name and nothing else, from the VCD
/// header, from the FST scope tree, and from the XTrace signal table.  The
/// unfiltered run comes first so a fixture that never had the signal cannot
/// pass by accident.
#[test]
fn wave_suppress_cuts_module_instances_and_paths_out_of_every_sink() {
    let sv = tmp("src", "sv");
    let vcd = tmp("out", "vcd");
    let fst = tmp("out", "fst");
    let xt = tmp("out", "xtrace");

    // With no exclusion at all first, so the fixture is checked too: every
    // name the next run must lose is really there to lose.
    let base = dump(&sv, &vcd, &fst, &xt, &[]);
    let all = vcd_names(&vcd);
    for n in ABSENT.iter().chain(PRESENT.iter()) {
        assert!(
            all.contains(*n),
            "the unfiltered dump has no `{n}`; the fixture is wrong"
        );
    }

    let mut args = suppress_args(SUPPRESS);
    args.push("--fst");
    let log = dump(&sv, &vcd, &fst, &xt, &args);

    let got = vcd_names(&vcd);
    for n in ABSENT {
        assert!(
            !got.contains(*n),
            "`{n}` survived the exclusion list in the VCD"
        );
    }
    for n in PRESENT {
        assert!(
            got.contains(*n),
            "`{n}` was dropped from the VCD but nothing excluded it"
        );
    }

    // One list, one result, in the two sinks that carry a hierarchy of their
    // own: an exclusion that stopped at the VCD header would leave the same
    // subtree sitting in the FST scope tree or the XTrace signal table.
    for (what, names) in [("FST", fst_names(&fst)), ("XTrace", xtrace_names(&xt))] {
        for n in ABSENT {
            assert!(
                !names.contains(*n),
                "`{n}` survived the exclusion list in the {what}"
            );
        }
        for n in PRESENT {
            assert!(
                names.contains(*n),
                "`{n}` left the {what} but nothing excluded it"
            );
        }
    }

    // And the same matcher must have run for all three sinks, while the empty
    // list of the run above must have cost the dump nothing.
    assert!(
        log.contains("[VCD] Suppressed")
            && log.contains("[FST] Suppressed")
            && log.contains("[XTrace] Suppressed"),
        "the exclusion list did not run for every sink; log {log}"
    );
    assert!(
        !base.contains("Suppressed"),
        "an empty exclusion list suppressed something; log {base}"
    );

    // And the report says so per entry, spelling each the way it was read:
    // `probe` as a definition with many instances, the other two as scopes.
    assert!(
        log.contains("instance(s) of module 'probe'"),
        "no module report for `probe`; log {log}"
    );
    assert!(
        log.contains("scope 'top.u_h2'"),
        "no scope report for `top.u_h2`; log {log}"
    );
    assert!(
        log.contains("scope 'u_h3.h'"),
        "no scope report for `u_h3.h`; log {log}"
    );
    assert!(
        !log.contains("matched no dumped signal"),
        "an entry that matched nothing was reported; log {log}"
    );

    for p in [&sv, &vcd, &fst, &xt] {
        let _ = fs::remove_file(p);
    }
}

#[test]
fn an_explicit_include_scope_outranks_the_exclusion_list() {
    let sv = tmp("ra_src", "sv");
    let vcd = tmp("ra_out", "vcd");
    let fst = tmp("ra_out", "fst");
    let xt = tmp("ra_out", "xtrace");

    let mut args = suppress_args(SUPPRESS);
    args.push("--fst-scope");
    args.push("top.u_h2");
    args.push("--fst");
    dump(&sv, &vcd, &fst, &xt, &args);

    let f = fst_names(&fst);
    // The whitelist names u_h2, so it comes back even though the list excludes
    // it... while the module entry still reaches the probe nested inside it.
    assert!(f.contains("top.u_h2.h"), "re-admitted subtree is missing");
    assert!(
        !f.contains("top.u_h2.u_probe.s"),
        "the module entry must still apply inside a re-admitted scope"
    );
    // Everything the list excluded and nothing re-admitted stays excluded.
    assert!(!f.contains("top.u_h3.h"), "`u_h3.h` came back uninvited");

    for p in [&sv, &vcd, &fst, &xt] {
        let _ = fs::remove_file(p);
    }
}

#[test]
fn an_entry_that_matched_nothing_is_reported_and_harmless() {
    let sv = tmp("ty_src", "sv");
    let vcd = tmp("ty_out", "vcd");
    let fst = tmp("ty_out", "fst");
    let xt = tmp("ty_out", "xtrace");

    let log = dump(&sv, &vcd, &fst, &xt, &suppress_args(&["top.u_nope"]));
    assert!(
        log.contains("matched no dumped signal"),
        "a mistyped entry passed without a word; log {log}"
    );
    // Harmless: the dump is the whole design, exactly as if it were absent.
    let got = vcd_names(&vcd);
    for n in ABSENT.iter().chain(PRESENT.iter()) {
        assert!(got.contains(*n), "`{n}` is missing from an unfiltered dump");
    }

    for p in [&sv, &vcd, &fst, &xt] {
        let _ = fs::remove_file(p);
    }
}

/// `--wave-suppress-module` says out loud what a bare name already means, so
/// the reading can never be guessed: the entry is a MODULE definition, matched
/// against the definition of every instance on a signal's path, at any depth.
/// The `module:` marker that carries that intent through the matcher is an
/// internal spelling and must not leak into what the user is told.
#[test]
fn a_forced_module_entry_reaches_every_instance_and_reports_as_one() {
    let sv = tmp("md_src", "sv");
    let vcd = tmp("md_out", "vcd");
    let fst = tmp("md_out", "fst");
    let xt = tmp("md_out", "xtrace");

    let mut args = vec!["--wave-suppress-module", "probe"];
    args.push("--fst");
    let log = dump(&sv, &vcd, &fst, &xt, &args);

    // The definition's own net, and the same nets of every instance of it â
    // directly under top and one level down inside each of the three holders.
    const EVERYWHERE: &[&str] = &[
        "top.u_p1.s",
        "top.u_p1.v",
        "top.u_h1.u_probe.s",
        "top.u_h1.u_probe.v",
        "top.u_h2.u_probe.s",
        "top.u_h2.u_probe.v",
        "top.u_h3.u_probe.s",
        "top.u_h3.u_probe.v",
    ];
    // What the OTHER entries of `SUPPRESS` would have taken and this run does
    // not list: those nets stay.
    const KEPT: &[&str] = &[
        "top.u_h2.h",
        "top.u_h3.h",
        "top.u_h1.h",
        "top.clk",
        "top.qa",
    ];
    for (what, names) in [("VCD", vcd_names(&vcd)), ("FST", fst_names(&fst))] {
        for n in EVERYWHERE {
            assert!(
                !names.contains(*n),
                "`{n}` survived a module entry in the {what}"
            );
        }
        for n in KEPT {
            assert!(names.contains(*n), "`{n}` left the {what} unasked");
        }
    }
    assert!(
        log.contains("instance(s) of module 'probe'"),
        "no module report for the forced entry; log {log}"
    );
    assert!(
        !log.contains("'module:probe'") && !log.contains("module:probe "),
        "the internal `module:` marker leaked into the report; log {log}"
    );
    assert!(
        !log.contains("matched no dumped signal"),
        "the forced entry matched nothing; log {log}"
    );

    for p in [&sv, &vcd, &fst, &xt] {
        let _ = fs::remove_file(p);
    }
}

/// A generated instance is named by its INDEX â `g_lane[1].u_bank` â which is
/// the spelling the signal table carries and the one a `$dumpvars` argument now
/// survives in (#1103). The exclusion list matches those names like any other:
/// the module reading must reach inside EVERY lane, and an indexed path must
/// take exactly ONE lane and leave its siblings whole â the `g_lane[1]` vs
/// `g_lane[10]` class of near miss, one index at a time.
#[test]
fn a_generated_instance_is_excluded_by_module_and_by_indexed_path() {
    // One lane's bank, named by its index.
    let sv = tmp("gn_src", "sv");
    let vcd = tmp("gn_one", "vcd");
    let fst = tmp("gn_one", "fst");
    let xt = tmp("gn_one", "xtrace");
    let mut args = suppress_args(&["top.g_lane[1].u_bank"]);
    args.push("--fst");
    let log = dump(&sv, &vcd, &fst, &xt, &args);
    assert!(
        !log.contains("matched no dumped signal"),
        "the indexed path entry matched nothing; log {log}"
    );
    for (what, names) in [
        ("VCD", vcd_names(&vcd)),
        ("FST", fst_names(&fst)),
        ("XTrace", xtrace_names(&xt)),
    ] {
        for n in ["top.g_lane[1].u_bank.m", "top.g_lane[1].u_bank.g"] {
            assert!(
                !names.contains(n),
                "`{n}` survived an indexed path entry in the {what}"
            );
        }
        // The two sibling lanes, and the lane-local net of the lane that lost
        // its bank: none of them sits under `g_lane[1].u_bank`.
        for n in [
            "top.g_lane[0].u_bank.m",
            "top.g_lane[2].u_bank.m",
            "top.g_lane[0].qw",
            "top.g_lane[1].qw",
            "top.g_lane[2].qw",
        ] {
            assert!(names.contains(n), "`{n}` left the {what} unasked");
        }
    }
    for p in [&sv, &vcd, &fst, &xt] {
        let _ = fs::remove_file(p);
    }

    // The module reading of the same shape: every lane loses its bank, while
    // the net the generate block itself declares stays.
    let sv = tmp("gn2_src", "sv");
    let vcd = tmp("gn_all", "vcd");
    let fst = tmp("gn_all", "fst");
    let xt = tmp("gn_all", "xtrace");
    let mut args = suppress_args(&["bank"]);
    args.push("--fst");
    let log = dump(&sv, &vcd, &fst, &xt, &args);
    assert!(
        log.contains("instance(s) of module 'bank'"),
        "no module report for the generated instances; log {log}"
    );
    for (what, names) in [
        ("VCD", vcd_names(&vcd)),
        ("FST", fst_names(&fst)),
        ("XTrace", xtrace_names(&xt)),
    ] {
        for i in 0..3 {
            for leaf in ["m", "g"] {
                let n = format!("top.g_lane[{i}].u_bank.{leaf}");
                assert!(
                    !names.contains(n.as_str()),
                    "`{n}` survived a module entry in the {what}"
                );
            }
            let kept = format!("top.g_lane[{i}].qw");
            assert!(
                names.contains(kept.as_str()),
                "lane {i} lost its own net to a module entry in the {what}"
            );
        }
    }

    for p in [&sv, &vcd, &fst, &xt] {
        let _ = fs::remove_file(p);
    }
}
