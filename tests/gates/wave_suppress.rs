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
    dump_of(SRC, sv, vcd, fst, xt, extra)
}

/// `dump` for another source text.
fn dump_of(
    src: &str,
    sv: &std::path::Path,
    vcd: &std::path::Path,
    fst: &std::path::Path,
    xt: &std::path::Path,
    extra: &[&str],
) -> String {
    fs::write(sv, src.replace("@VCD@", &vcd.to_string_lossy())).unwrap();
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

/// A second design for the entry shapes the first has none of: unpacked
/// array variables, an instance array, a generate loop holding a variable,
/// and an instance whose name is also a module name.
const SRC_ARRAYS: &str = r#"
`timescale 1ns/1ps
module ram(input logic clk, input logic [1:0] a, input logic [7:0] d, output logic [7:0] q);
  logic [7:0] mem [0:3];
  logic [7:0] rq;
  assign q = rq;
  always @(posedge clk) begin mem[a] <= d; rq <= mem[a]; end
endmodule
module leaf(input logic clk, input logic [7:0] d, output logic [7:0] q);
  logic [7:0] r;
  logic [3:0] tbl [0:1][0:1];
  always @(posedge clk) begin r <= d + 8'd1; tbl[d[0]][d[1]] <= d[3:0]; end
  assign q = r;
endmodule
module mid(input logic clk, input logic [7:0] d, output logic [7:0] q);
  logic [7:0] w;
  leaf u_leaf(.clk(clk), .d(d), .q(w));
  ram  u_ram(.clk(clk), .a(d[1:0]), .d(w), .q(q));
endmodule
module top;
  logic clk = 1'b0;
  logic [7:0] din = 8'd0;
  logic [7:0] q0, q1;
  logic [7:0] tmem [0:3];
  logic [7:0] tmem2 [0:1];
  always #5 clk = ~clk;
  always @(posedge clk) begin din <= din + 8'd3; tmem[din[1:0]] <= din; tmem2[din[0]] <= din; end
  mid  u_mid(.clk(clk), .d(din), .q(q0));
  leaf u_leaf(.clk(clk), .d(din), .q(q1));
  leaf u_arr [1:0] (.clk(clk), .d(din), .q());
  leaf u_arr2 [1:0] (.clk(clk), .d(din), .q());
  genvar gi;
  for (gi = 0; gi < 2; gi++) begin : g
    logic [7:0] gw;
    ram u_ram(.clk(clk), .a(din[1:0]), .d(din), .q(gw));
  end
  initial begin
    $dumpfile("@VCD@");
    $dumpvars(0, top);
    #60;
    $finish;
  end
endmodule
"#;

/// The names of all three sinks for one run of `SRC_ARRAYS`, with xezim's
/// stderr. `stem` keeps the files of concurrently running tests apart.
fn dump_arrays(stem: &str, extra: &[&str]) -> ([HashSet<String>; 3], String) {
    let sv = tmp(&format!("{stem}_src"), "sv");
    let vcd = tmp(stem, "vcd");
    let fst = tmp(stem, "fst");
    let xt = tmp(stem, "xtrace");
    let mut args = extra.to_vec();
    args.push("--fst");
    let log = dump_of(SRC_ARRAYS, &sv, &vcd, &fst, &xt, &args);
    let names = [vcd_names(&vcd), fst_names(&fst), xtrace_names(&xt)];
    for p in [&sv, &vcd, &fst, &xt] {
        let _ = fs::remove_file(p);
    }
    (names, log)
}

const SINKS: [&str; 3] = ["VCD", "FST", "XTrace"];

fn assert_names(names: &[HashSet<String>; 3], absent: &[&str], present: &[&str]) {
    for (set, sink) in names.iter().zip(SINKS) {
        for n in absent {
            assert!(!set.contains(*n), "`{n}` survived in the {sink}");
        }
        for n in present {
            assert!(set.contains(*n), "`{n}` is missing from the {sink}");
        }
    }
}

/// A path entry that names an unpacked array variable takes every element
/// of it, and one that names an instance array or a generate loop takes
/// every element instance. A name that only starts the same is untouched.
#[test]
fn a_path_entry_covers_array_elements_instance_arrays_and_generate_loops() {
    let (all, _) = dump_arrays("ar_full", &[]);
    let gone = [
        "top.tmem[0]",
        "top.tmem[3]",
        "top.u_mid.u_ram.mem[2]",
        "top.u_arr[0].r",
        "top.u_arr[1].tbl[1][0]",
        "top.g[0].gw",
        "top.g[1].u_ram.rq",
        "top.u_leaf.tbl[1][0]",
        "top.u_leaf.tbl[1][1]",
    ];
    let kept = [
        "top.tmem2[0]",
        "top.u_mid.u_ram.rq",
        "top.u_arr2[0].r",
        "top.u_leaf.tbl[0][1]",
        "top.u_leaf.r",
        "top.clk",
    ];
    assert_names(&all, &[], &gone);
    assert_names(&all, &[], &kept);
    let (names, log) = dump_arrays(
        "ar_cut",
        &suppress_args(&[
            "top.tmem",
            "u_mid.u_ram.mem",
            "top.u_arr",
            "top.g",
            "top.u_leaf.tbl[1]",
        ]),
    );
    assert_names(&names, &gone, &kept);
    assert!(!log.contains("matched no dumped signal"), "log {log}");
    assert!(
        log.contains("[VCD] Suppressed 4 signals under scope 'top.tmem'"),
        "log {log}"
    );
}

/// A bare name that is no module definition is a path below the top module,
/// as a bare `--fst-scope` name is. A bare name that is a module stays one,
/// even when an instance carries it too.
#[test]
fn a_bare_name_that_is_no_module_is_a_path_below_the_top() {
    let (names, log) = dump_arrays("bn_inst", &suppress_args(&["u_mid", "tmem2"]));
    assert_names(
        &names,
        &["top.u_mid.w", "top.u_mid.u_leaf.r", "top.tmem2[1]"],
        &["top.u_leaf.r", "top.tmem[0]", "top.q0"],
    );
    assert!(!log.contains("matched no dumped signal"), "log {log}");
    let (names, log) = dump_arrays("bn_mod", &suppress_args(&["leaf"]));
    assert_names(
        &names,
        &["top.u_leaf.r", "top.u_mid.u_leaf.r", "top.u_arr[1].r"],
        &["top.u_mid.w", "top.u_mid.u_ram.rq"],
    );
    assert!(
        log.contains("under 6 instance(s) of module 'leaf'"),
        "log {log}"
    );
}

/// An include scope inside an excluded subtree brings back what it names,
/// not the rest of the subtree that another include scope selects.
#[test]
fn an_include_scope_brings_back_only_what_it_names() {
    let mut args = suppress_args(&["top.u_mid.u_ram"]);
    args.extend([
        "--fst-scope",
        "top.u_mid",
        "--fst-scope",
        "top.u_mid.u_ram.rq",
    ]);
    let (names, _) = dump_arrays("rd_one", &args);
    let fst = &names[1];
    assert!(fst.contains("top.u_mid.u_ram.rq"), "{fst:?}");
    assert!(fst.contains("top.u_mid.w"), "{fst:?}");
    assert!(!fst.contains("top.u_mid.u_ram.q"), "{fst:?}");
    assert!(!fst.contains("top.u_mid.u_ram.mem[0]"), "{fst:?}");
    // An element of an excluded array, asked for by name.
    let mut args = suppress_args(&["top.g"]);
    args.extend(["--fst-scope", "top.g[1]"]);
    let (names, _) = dump_arrays("rd_elem", &args);
    let fst = &names[1];
    assert!(fst.contains("top.g[1].gw"), "{fst:?}");
    assert!(!fst.contains("top.g[0].gw"), "{fst:?}");
}

/// What the report says: the top module by name, and no "matched nothing"
/// for an entry that lies inside what another entry already excluded.
#[test]
fn the_report_names_the_top_module_and_skips_covered_entries() {
    let sv = tmp("rp_top_src", "sv");
    let vcd = tmp("rp_top", "vcd");
    let xt = tmp("rp_top", "xtrace");
    let log = dump_of(SRC_ARRAYS, &sv, &vcd, &vcd, &xt, &suppress_args(&["top"]));
    let names = vcd_names(&vcd);
    assert!(names.is_empty(), "{names:?}");
    assert!(
        log.contains("signals under the top module 'top'"),
        "log {log}"
    );
    for p in [&sv, &vcd, &xt] {
        let _ = fs::remove_file(p);
    }
    let (_, log) = dump_arrays(
        "rp_cov",
        &suppress_args(&["leaf", "top.u_leaf.r", "top.u_mid"]),
    );
    assert!(!log.contains("matched no dumped signal"), "log {log}");
    let (_, log) = dump_arrays("rp_none", &[]);
    assert!(!log.contains("Suppressed"), "log {log}");
}

/// The list applies to the dumps of a compiled artifact, given when it runs.
#[test]
fn a_compiled_artifact_takes_the_exclusion_list() {
    let sv = tmp("art_src", "sv");
    let vcd = tmp("art", "vcd");
    let art = tmp("art", "art");
    fs::write(&sv, SRC_ARRAYS.replace("@VCD@", &vcd.to_string_lossy())).unwrap();
    let bin = env!("CARGO_BIN_EXE_xezim");
    let o = Command::new(bin)
        .arg("--compile")
        .arg(&sv)
        .args(["-s", TOP, "-o"])
        .arg(&art)
        .output()
        .expect("compile");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let o = Command::new(bin)
        .arg(&art)
        .args([
            "--wave",
            "--wave-suppress",
            "ram",
            "--wave-suppress",
            "top.tmem",
        ])
        .output()
        .expect("run");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let got = vcd_names(&vcd);
    assert!(!got.contains("top.u_mid.u_ram.rq"), "{got:?}");
    assert!(!got.contains("top.g[0].u_ram.mem[1]"), "{got:?}");
    assert!(!got.contains("top.tmem[2]"), "{got:?}");
    assert!(got.contains("top.u_mid.w"), "{got:?}");
    for p in [&sv, &vcd, &art] {
        let _ = fs::remove_file(p);
    }
}

/// Two runs with the same list write the same dump: the VCD apart from its
/// `$date`, the XTrace byte for byte, and the same FST hierarchy.
#[test]
fn a_suppressed_dump_is_the_same_on_every_run() {
    let run = |stem: &str| {
        let sv = tmp("det_src", "sv");
        let vcd = tmp(stem, "vcd");
        let fst = tmp(stem, "fst");
        let xt = tmp(stem, "xtrace");
        let mut args = suppress_args(&["ram", "u_arr", "top.tmem"]);
        args.push("--fst");
        dump_of(SRC_ARRAYS, &sv, &vcd, &fst, &xt, &args);
        let text = fs::read_to_string(&vcd)
            .expect("vcd")
            // Every run writes to its own file, named in no record.
            .lines()
            .skip_while(|l| !l.starts_with("$version"))
            .collect::<Vec<_>>()
            .join("\n");
        let out = (text, fst_names(&fst), fs::read(&xt).expect("xtrace"));
        for p in [&sv, &vcd, &fst, &xt] {
            let _ = fs::remove_file(p);
        }
        out
    };
    let a = run("det_a");
    let b = run("det_b");
    assert!(a.0.contains("$var"), "{}", a.0);
    assert_eq!(a.0, b.0, "the VCD differs between two runs");
    assert_eq!(a.1, b.1, "the FST hierarchy differs between two runs");
    assert_eq!(a.2, b.2, "the XTrace differs between two runs");
}
