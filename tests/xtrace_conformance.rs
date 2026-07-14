//! XTrace v1.0 (XTrace_Specification_v1_0.txt) — CONFORMANCE LEVEL 0.
//!
//! §24 Level 0 is "dictionary + time + signal deltas". This suite pins exactly
//! that, plus the one §10.4 record family we add (events — an SV `event` has no
//! level, so no signal delta can carry it). It does NOT test the semantic layer
//! (transactions, assertions, enums, source maps, memory records): we do not
//! emit it, and the header does not claim it.
//!
//! Before these fixes:
//!   §6.7-6.9   the header dropped `@capabilities`, `@compression` and
//!              `@extensions` outright. A `.zst` dump was zstd-framed while its
//!              own header said nothing at all about compression — a consumer
//!              could not recover.
//!   §9.2       `S` records carried neither `enc=` nor `width=`, so a consumer
//!              had to guess the encoding and infer the width from the type name
//!              (impossible for `bit`/`real`). A `logic [15:8]` silently
//!              renumbered its bits to [7:0].
//!   §9.3/§15.1 a `real` was typed `s64` and emitted as its raw IEEE-754 bit
//!              pattern (`real r = 3.25` → `0x400a000000000000`), which a
//!              consumer decodes as the integer -4609434218613702656.
//!   §10.4      an `event` was traced as a toggling 1-bit LEVEL, so three
//!              `->e1` triggers emitted 0x1, 0x0, 0x1 — the second one reads as
//!              "no event" — and two triggers in one time slot cancelled out.
//!   §19.2      port-connected nets emitted TWO `S` records with the SAME
//!              signal_id ("once introduced, an ID retains its meaning"), so a
//!              parser keyed on the id clobbered one of them.
//!   §10.1/19.3 the trace stopped at the last CHANGE, not at the end of the run
//!              (a run to t=40 whose last toggle was at t=30 ended at 30).
//!   §15.3      a vector with any x/z bit was written FULL WIDTH in binary
//!              (`0bZZZZ...`), making XTrace LARGER than the VCD it replaces.
//!
//! Each test runs a design through the library (or the CLI, for flag-driven
//! behaviour) and asserts on the XTrace TEXT.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

// ───────────────────────────── harness ─────────────────────────────

fn temp_path(tag: &str, ext: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "xezim_xtrace_{}_{}_{}.{}",
        tag,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        ext
    ));
    path
}

/// Run `src` through the library with an XTrace dump enabled, and return the
/// trace text. Every trace produced here is also run through `validate` (below)
/// so the §18 grammar is checked on EVERY test's output, not just one.
fn dump(tag: &str, src: &str) -> String {
    dump_scoped(tag, src, &[])
}

fn dump_scoped(tag: &str, src: &str, scopes: &[String]) -> String {
    let path = temp_path(tag, "xt");
    let sim = xezim::simulate_multi(
        &[src.to_string()],
        1_000_000,
        None,
        &[],
        &[],
        None,
        false,
        None,
        None,
        &[],
        &[],
        1,
        Some(path.to_str().unwrap()),
        scopes,
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
    );
    sim.expect("simulate failed");
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("no XTrace written to {}: {}", path.display(), e));
    let _ = fs::remove_file(&path);
    validate(&text);
    text
}

/// The `S` (§9.2) dictionary record whose NAME field is `name`.
fn sig_line(xt: &str, name: &str) -> String {
    xt.lines()
        .find(|l| l.starts_with("S,") && l.split(',').nth(3) == Some(name))
        .unwrap_or_else(|| panic!("no S record for `{}` in:\n{}", name, xt))
        .to_string()
}

/// Every `S` record whose NAME field is `name` — the same leaf appears once per
/// instance that declares it (`din` in `u_mid` and in `u_mid.u_leaf`).
fn sig_lines_all(xt: &str, name: &str) -> Vec<String> {
    xt.lines()
        .filter(|l| l.starts_with("S,") && l.split(',').nth(3) == Some(name))
        .map(str::to_string)
        .collect()
}

/// The signal_id assigned to `name`.
fn sid_of(xt: &str, name: &str) -> String {
    sig_line(xt, name).split(',').nth(1).unwrap().to_string()
}

/// Every value `name` takes in the trace, in order: its `N,full` seed followed
/// by each `D`/`P` delta that mentions its id.
fn values_of(xt: &str, name: &str) -> Vec<String> {
    let sid = sid_of(xt, name);
    let mut out = Vec::new();
    for line in trace_section(xt).lines() {
        let f: Vec<&str> = line.split(',').collect();
        match f.first() {
            Some(&"D") if f.len() == 3 && f[1] == sid => out.push(f[2].to_string()),
            Some(&"P") | Some(&"N") => {
                let skip = if f[0] == "N" { 2 } else { 1 };
                for tok in &f[skip..] {
                    if let Some((s, v)) = tok.split_once('=') {
                        if s == sid {
                            out.push(v.to_string());
                        }
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// The body of `@section trace`.
fn trace_section(xt: &str) -> String {
    let start = xt.find("@section trace").expect("no @section trace");
    let rest = &xt[start..];
    let end = rest.find("@section end").unwrap_or(rest.len());
    rest[..end].to_string()
}

/// The §18 grammar, checked in-process on every trace this suite produces:
/// header directives are known/unique and precede the sections, the sections run
/// dict → trace → end, every referenced signal_id and module_id is DECLARED, no
/// signal_id is declared twice (§19.2), every `alias=` resolves to a declared
/// non-alias id, and no alias carries a value delta of its own (§9.2).
///
/// (The same checks, in Python, accept both upstream xezim-0.1.2 reference
/// traces — the grammar here is not tailored to our own output.)
fn validate(xt: &str) {
    let mut section = String::new();
    let mut sections: Vec<String> = Vec::new();
    let mut directives: Vec<String> = Vec::new();
    let mut signals: Vec<String> = Vec::new();
    let mut modules: Vec<String> = Vec::new();
    let mut aliases: Vec<(String, String)> = Vec::new();

    for (n, raw) in xt.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue; // §8.3 comments and blank lines
        }
        if let Some(name) = line.strip_prefix("@section ") {
            sections.push(name.to_string());
            section = name.to_string();
            continue;
        }
        if line.starts_with('@') {
            let d = line.split_whitespace().next().unwrap().to_string();
            assert!(
                section.is_empty(),
                "§6: directive {} after @section {} (line {})",
                d,
                section,
                n + 1
            );
            assert!(!directives.contains(&d), "§6: duplicate directive {}", d);
            if directives.is_empty() {
                assert_eq!(d, "@xtrace", "§6.1: the first directive must be @xtrace");
            }
            directives.push(d);
            continue;
        }
        let f: Vec<&str> = line.split(',').collect();
        match (section.as_str(), f[0]) {
            ("dict", "M") => {
                assert_eq!(f.len(), 3, "§9.1: M,<module_id>,<hier_path> — {}", line);
                assert!(
                    !modules.contains(&f[1].to_string()),
                    "§19.2: duplicate module_id {}",
                    f[1]
                );
                modules.push(f[1].to_string());
            }
            ("dict", "S") => {
                assert!(f.len() >= 5, "§9.2: S,<sid>,<mid>,<name>,<type> — {}", line);
                assert!(
                    !signals.contains(&f[1].to_string()),
                    "§19.2 'once introduced, an ID retains its meaning': \
                     duplicate signal_id {} — {}",
                    f[1],
                    line
                );
                assert!(
                    modules.contains(&f[2].to_string()),
                    "§9.2: S references undeclared module_id {}",
                    f[2]
                );
                signals.push(f[1].to_string());
                for kv in &f[5..] {
                    let (k, v) = kv.split_once('=').unwrap_or_else(|| {
                        panic!("§9.2: malformed attribute {:?} in {}", kv, line)
                    });
                    if k == "alias" {
                        aliases.push((f[1].to_string(), v.to_string()));
                    }
                }
            }
            ("trace", "T") => assert!(
                f.len() >= 2 && f[1].starts_with('+') && f[1][1..].parse::<u64>().is_ok(),
                "§10.1: T,+<delta> — {}",
                line
            ),
            ("trace", "D") => {
                assert_eq!(f.len(), 3, "§10.2: D,<signal_id>,<value> — {}", line);
                check_delta(&signals, &aliases, f[1], f[2], line);
            }
            ("trace", "P") | ("trace", "N") => {
                let skip = if f[0] == "N" { 2 } else { 1 };
                assert!(f.len() > skip, "§10.3/§10.6: no assignments — {}", line);
                for tok in &f[skip..] {
                    let (s, v) = tok
                        .split_once('=')
                        .unwrap_or_else(|| panic!("expected <sid>=<value> — {}", line));
                    check_delta(&signals, &aliases, s, v, line);
                }
            }
            ("trace", "X") => {
                assert!(f.len() >= 2 && !f[1].is_empty(), "§10.4: X,<event_type>");
                for kv in &f[2..] {
                    let (k, v) = kv
                        .split_once('=')
                        .unwrap_or_else(|| panic!("§10.4: malformed attribute — {}", line));
                    if k == "sig" {
                        assert!(
                            signals.contains(&v.to_string()),
                            "§19.2: X references undeclared signal_id {}",
                            v
                        );
                    }
                }
            }
            ("end", rec) => panic!("§7: record {:?} after '@section end'", rec),
            (sec, rec) => panic!("§7/§10: record {:?} in section {:?}", rec, sec),
        }
    }
    assert_eq!(
        sections.first().map(String::as_str),
        Some("dict"),
        "§7: the first section must be dict"
    );
    assert_eq!(
        sections.last().map(String::as_str),
        Some("end"),
        "§7: the last section must be end"
    );
    assert_eq!(
        sections,
        vec!["dict", "trace", "end"],
        "§7: sections must run dict → trace-like → end"
    );
    for (sid, canon) in &aliases {
        assert!(
            signals.contains(canon),
            "§9.2: alias {} → undeclared signal_id {}",
            sid,
            canon
        );
        assert!(
            !aliases.iter().any(|(s, _)| s == canon),
            "§9.2: alias {} → {} which is itself an alias",
            sid,
            canon
        );
    }
}

fn check_delta(signals: &[String], aliases: &[(String, String)], sid: &str, val: &str, line: &str) {
    assert!(
        signals.contains(&sid.to_string()),
        "§19.2: delta references undeclared signal_id {} — {}",
        sid,
        line
    );
    // §9.2: an alias NAMES a net; it is not a second net. Only the canonical id
    // may carry deltas, or a consumer sees one wire as two independent signals.
    assert!(
        !aliases.iter().any(|(s, _)| s == sid),
        "§9.2: value delta emitted for alias {} — {}",
        sid,
        line
    );
    assert!(!val.is_empty(), "§15: empty value — {}", line);
    let ok = val == "X"
        || val == "Z"
        || val.starts_with("0x")
        || val.starts_with("0b")
        || val.parse::<f64>().is_ok();
    assert!(ok, "§15: malformed value {:?} — {}", val, line);
}

/// A design exercising every Level-0 record shape at once.
const KITCHEN: &str = r#"
module leaf(input logic [7:0] din, output logic [7:0] dout);
    assign dout = din + 8'h01;
endmodule
module mid(input logic [7:0] din, output logic [7:0] dout);
    leaf u_leaf(.din(din), .dout(dout));
endmodule
module top;
    logic clk;
    logic [15:8] hi;
    logic [7:0]  lo;
    real         r;
    event        e1;
    logic [7:0]  src_bus;
    logic [7:0]  sink_bus;
    logic [3:0]  xz;
    logic [7:0]  allx;
    logic [7:0]  allz;
    logic [7:0]  mem [0:3];
    mid u_mid(.din(src_bus), .dout(sink_bus));
    initial begin
        clk = 0; hi = 8'h00; lo = 8'h00; r = 0.0; src_bus = 8'h10;
        allx = 8'hxx; allz = 8'hzz; xz = 4'b01xz;
        mem[0] = 8'h00; mem[1] = 8'h00; mem[2] = 8'h00; mem[3] = 8'h00;
    end
    // The clock stops at t=30; the run ends at t=40.
    initial begin
        #5 clk = 1; #5 clk = 0; #5 clk = 1;
        #5 clk = 0; #5 clk = 1; #5 clk = 0;
    end
    initial begin
        #10 r = 3.25; hi = 8'ha5; src_bus = 8'h20; mem[1] = 8'hbe; -> e1;
        #10 r = -0.5; allx = 8'h00; mem[2] = 8'hef; -> e1;
        #10 allz = 8'h00; src_bus = 8'h30; -> e1;
        #10 $finish;
    end
endmodule
"#;

// ───────────────────────── §6 header directives ─────────────────────────

/// §6.1-6.9 + §18: the header carries every directive the grammar defines, each
/// once, all before the first `@section`, and the sections run dict → trace →
/// end. `@capabilities`, `@compression` and `@extensions` were dropped entirely
/// by the regressed writer.
#[test]
fn header_directives_are_complete_and_ordered() {
    let xt = dump("header", KITCHEN);
    let head: Vec<&str> = xt
        .lines()
        .take_while(|l| !l.starts_with("@section"))
        .filter(|l| l.starts_with('@'))
        .collect();
    assert_eq!(
        head,
        vec![
            "@xtrace 1.1",
            "@format text",
            &format!("@producer xezim {}", env!("CARGO_PKG_VERSION")),
            "@timescale 1ns",
            "@design top",
            "@profile minimal",
            "@capabilities signal_delta|events",
            "@compression none",
            "@extensions ignore_unknown",
        ],
        "§6: header directives"
    );
    // §8.3: the signal-selection policy is recorded as a comment.
    assert!(xt.contains("# xtrace-signals "), "§8.3 policy comment: {}", xt);
    // §7: sections, in order.
    let order: Vec<&str> = xt
        .lines()
        .filter(|l| l.starts_with("@section"))
        .collect();
    assert_eq!(order, vec!["@section dict", "@section trace", "@section end"]);
    // §24: the trace opens with the Level-0 t=0 checkpoint.
    assert!(xt.contains("\nT,+0\n"), "§10.1: no T,+0");
    assert!(xt.contains("\nN,full,"), "§10.6: no N,full snapshot");
    // We are Level 0 + events: nothing from the semantic/transactional layer.
    for rec in ["Q,", "QS,", "A,", "E,", "TQ,", "MW,", "MR,", "MB,", "N,ctx"] {
        assert!(
            !xt.lines().any(|l| l.starts_with(rec)),
            "§24: Level 0 must not emit {:?} records",
            rec
        );
    }
    assert!(!xt.contains("X,sim_telemetry"));
    // §6.5: `minimal` is the truth. Never claim the Appendix-A AI profile.
    assert!(!xt.contains("xezim_ai_debug"), "§6.5: profile must not over-claim");
}

/// §6.5/§24: `--xtrace-level 1` (the semantic layer) is NOT implemented. It must
/// warn and degrade to Level 0 rather than emit a header that lies. `@profile`
/// is settable (the 0.1.2 reference traces say `raw_delta`) and defaults to the
/// accurate `minimal`.
#[test]
fn xtrace_level_one_is_reserved_and_profile_is_settable() {
    let src = temp_path("reserved", "sv");
    let trace = temp_path("reserved", "xt");
    fs::write(&src, KITCHEN).unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--simulate", "-s", "top", "--max-time", "100", "--xtrace"])
        .arg(&trace)
        .args(["--xtrace-level", "1", "--xtrace-profile", "raw_delta"])
        .arg(&src)
        .output()
        .expect("run xezim");
    assert!(out.status.success(), "xezim failed: {:?}", out);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--xtrace-level 1 is reserved"),
        "§24: a reserved level must warn, got: {}",
        stderr
    );

    let xt = fs::read_to_string(&trace).expect("read xtrace");
    let _ = fs::remove_file(&src);
    let _ = fs::remove_file(&trace);
    validate(&xt);
    assert!(xt.contains("@profile raw_delta"), "§6.5: --xtrace-profile");
    assert!(!xt.contains("X,sim_telemetry"));
    // Level 1 was asked for and refused — the payload is still Level 0.
    assert!(!xt.lines().any(|l| l.starts_with("Q,") || l.starts_with("A,")));
}

/// §6.8: a zstd-framed file MUST say so. The regressed writer compressed the
/// stream whenever the name ended in `.zst` and left `@compression` out
/// altogether, which no consumer can recover from.
#[test]
fn compression_is_declared_in_the_header() {
    let src = temp_path("zstd", "sv");
    let trace = temp_path("zstd", "xt.zst");
    fs::write(&src, KITCHEN).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--simulate", "-s", "top", "--max-time", "100", "--xtrace"])
        .arg(&trace)
        .arg(&src)
        .output()
        .expect("run xezim");
    assert!(out.status.success(), "xezim failed: {:?}", out);

    let raw = fs::read(&trace).expect("read .zst");
    let _ = fs::remove_file(&src);
    let _ = fs::remove_file(&trace);
    assert_eq!(
        &raw[..4],
        &[0x28, 0xB5, 0x2F, 0xFD],
        "the file really is a zstd frame"
    );
    let xt = String::from_utf8(zstd::stream::decode_all(&raw[..]).expect("zstd decode")).unwrap();
    validate(&xt);
    assert!(xt.contains("@compression zstd"), "§6.8: {}", &xt[..200]);
    assert!(
        xt.contains("@capabilities signal_delta|events|compression_zstd"),
        "§6.7: the capability list must mention the transport"
    );
    // An uncompressed dump of the same design says `none`, not nothing.
    assert!(dump("nocomp", KITCHEN).contains("@compression none"));
}

// ───────────────────────── §9.2 signal records ─────────────────────────

/// §9.2: every `S` record carries `enc=` and `width=`. Without them a consumer
/// cannot tell how values are encoded, and cannot recover the width of a `bit`
/// or a `real` at all.
#[test]
fn signal_records_carry_enc_and_width() {
    let xt = dump("attrs", KITCHEN);
    let dict: Vec<&str> = xt.lines().filter(|l| l.starts_with("S,")).collect();
    assert!(!dict.is_empty());
    for line in &dict {
        assert!(line.contains(",width="), "§9.2: no width= on {}", line);
        assert!(line.contains(",enc="), "§9.2: no enc= on {}", line);
    }
    assert!(sig_line(&xt, "clk").starts_with("S,"));
    assert!(
        sig_line(&xt, "clk").contains(",clk,bit,enc=delta,width=1"),
        "§9.2/§9.3: {}",
        sig_line(&xt, "clk")
    );
    assert!(
        sig_line(&xt, "lo").contains(",lo,u8,enc=delta,width=8"),
        "§9.2: {}",
        sig_line(&xt, "lo")
    );
}

/// §9.2 + §8.7: a bit-range whose LSB is not 0 (`logic [15:8] hi`) has no home
/// in `width=8` — the bits silently renumber to [7:0]. §8.7 lets a producer add
/// an attribute a consumer may ignore, so the range rides along in `range=`.
/// A plain `[7:0]` adds nothing and gets none.
#[test]
fn nonzero_lsb_bit_range_is_preserved() {
    let xt = dump("range", KITCHEN);
    let hi = sig_line(&xt, "hi");
    assert!(
        hi.contains(",width=8") && hi.contains(",range=15:8"),
        "§9.2/§8.7: [15:8] must keep its LSB offset — {}",
        hi
    );
    let lo = sig_line(&xt, "lo");
    assert!(
        !lo.contains("range="),
        "a [7:0] range is what width= already says — {}",
        lo
    );
}

/// §9.2 `alias=<canonical_signal_id>` + §19.2 "once introduced, an ID retains
/// its meaning". A net threaded through instance ports (`src_bus` →
/// `u_mid.din` → `u_mid.u_leaf.din`) is ONE net with THREE names. The regressed
/// writer gave all three the same signal_id — two `S` records, one id — so a
/// parser keyed on the id lost a name. Each name now has its OWN id, the
/// non-canonical ones say `alias=`, and only the canonical id carries deltas.
#[test]
fn aliased_port_nets_get_unique_ids() {
    let xt = dump("alias", KITCHEN);
    // §19.2: no duplicate ids anywhere (checked for every trace by `validate`).
    let ids: Vec<&str> = xt
        .lines()
        .filter(|l| l.starts_with("S,"))
        .map(|l| l.split(',').nth(1).unwrap())
        .collect();
    let mut uniq = ids.clone();
    uniq.sort_unstable();
    uniq.dedup();
    assert_eq!(ids.len(), uniq.len(), "§19.2: duplicate signal_id in\n{}", xt);

    let canon = sid_of(&xt, "src_bus");
    let dins = sig_lines_all(&xt, "din");
    assert_eq!(dins.len(), 2, "din is declared in mid and in leaf: {:?}", dins);
    for din in &dins {
        assert!(
            din.contains(&format!(",alias={}", canon)),
            "§9.2: a port-connected copy of src_bus must alias it — {}",
            din
        );
        let sid = din.split(',').nth(1).unwrap();
        assert_ne!(sid, canon, "§19.2: an alias needs its own id");
        for line in trace_section(&xt).lines() {
            assert!(
                !line.contains(&format!(",{}=", sid)) && !line.starts_with(&format!("D,{},", sid)),
                "§9.2: an alias must not carry value deltas — {}",
                line
            );
        }
    }
    // The canonical id does carry them.
    assert!(
        values_of(&xt, "src_bus").contains(&"0x20".to_string()),
        "the canonical net still emits its deltas"
    );
}

// ───────────────────────── §15 values ─────────────────────────

/// §9.3/§15.1: a `real` is a decimal number, not a bit pattern. The regressed
/// writer typed it `s64` and emitted the raw IEEE-754 bits, so `real r = 3.25`
/// came out as `0x400a000000000000` — the integer -4609434218613702656 to a
/// consumer. §9.3's type list is *recommended*, not exhaustive, so a `real`
/// type is a legitimate producer choice; §15.1 allows decimal wherever it is
/// "semantically better".
#[test]
fn real_values_are_decimal() {
    let xt = dump("real", KITCHEN);
    let r = sig_line(&xt, "r");
    assert!(
        r.contains(",r,real,enc=delta,width=64"),
        "§9.3: a real is typed `real`, 64 bits — {}",
        r
    );
    assert!(!r.contains(",s64"), "§9.3: a real is not a signed integer — {}", r);
    let vals = values_of(&xt, "r");
    assert_eq!(
        vals,
        vec!["0", "3.25", "-0.5"],
        "§15.1: reals must read back as decimals, got {:?}",
        vals
    );
    assert!(
        !xt.contains("0x400a000000000000"),
        "§15.1: the raw IEEE-754 bit pattern of 3.25 must not appear"
    );
}

/// §15.3: `X` and `Z` are legal compact spellings for an all-unknown value. A
/// mixed vector keeps FULL width: VCD's leading-run suppression is legal only
/// because §21.7.2.1 of the LRM defines a left-extension rule for a short value;
/// XTrace defines none, so a partially collapsed vector is unparseable.
#[test]
fn unknown_values_use_the_compact_forms() {
    let xt = dump("xz", KITCHEN);
    assert_eq!(
        values_of(&xt, "allx"),
        vec!["X", "0x0"],
        "§15.3: an all-x vector is `X`"
    );
    assert_eq!(
        values_of(&xt, "allz"),
        vec!["X", "Z", "0x0"],
        "§15.3: an all-z vector is `Z` (it starts life all-x)"
    );
    assert_eq!(
        values_of(&xt, "xz"),
        vec!["X", "0b01XZ"],
        "§15.3: a MIXED x/z vector keeps every bit"
    );
    assert!(
        !xt.contains("0bXXXXXXXX") && !xt.contains("0bZZZZZZZZ"),
        "§15.3: a uniform unknown must not be spelled out bit by bit"
    );
}

// ───────────────────────── §10.4 events ─────────────────────────

/// §10.4/§19.5: an SV `event` has no level. Tracing it as a 1-bit signal made
/// three `->e1` triggers emit 0x1, 0x0, 0x1 — the second trigger reads as "no
/// event" — and two triggers inside one time slot cancelled to nothing at all.
/// The spec has a record for exactly this: `X,<event_type>[,k=v]*`. We emit
/// `X,event,sig=<signal_id>` per trigger and declare the `events` capability.
///
/// (Repeat triggers of the SAME event within ONE time slot collapse to one `X`:
/// the simulator records a trigger TIME, not a count — the same model the VCD
/// `event` pulse uses.)
#[test]
fn events_are_x_records_not_toggling_levels() {
    let xt = dump("event", KITCHEN);
    let e1 = sig_line(&xt, "e1");
    assert!(
        e1.contains(",e1,event,enc=event"),
        "§9.3: an event is typed `event`, not `bit` — {}",
        e1
    );
    let sid = sid_of(&xt, "e1");

    // Three triggers, at t=10, t=20 and t=30 → three X records.
    let xs: Vec<&str> = xt.lines().filter(|l| l.starts_with("X,")).collect();
    assert_eq!(
        xs,
        vec![
            format!("X,event,sig={}", sid),
            format!("X,event,sig={}", sid),
            format!("X,event,sig={}", sid)
        ],
        "§10.4: one X record per trigger"
    );
    // …and NOT a single value delta: an event has no level to change.
    assert!(
        values_of(&xt, "e1").is_empty(),
        "§19.5: an event must not appear in N/D/P records"
    );
    // §6.7: the capability is declared, because we really do emit the family.
    assert!(xt.contains("@capabilities signal_delta|events"));
}

// ───────────────────────── §10.1 time ─────────────────────────

/// §10.1/§19.3: the trace must close at the END OF THE RUN, not at the last
/// value change. `KITCHEN` stops changing at t=30 and runs to `$finish` at
/// t=40; the regressed writer's trace simply ended at 30, and a consumer could
/// not tell a quiet tail from a truncated file. A lone trailing `T` is a
/// well-formed trace_record (§18) and says exactly that: time advanced, nothing
/// changed.
#[test]
fn trace_closes_at_the_final_simulation_time() {
    let xt = dump("tail", KITCHEN);
    let body = trace_section(&xt);
    let deltas: Vec<u64> = body
        .lines()
        .filter(|l| l.starts_with("T,+"))
        .map(|l| l.split(',').nth(1).unwrap()[1..].parse().unwrap())
        .collect();
    assert_eq!(
        deltas.iter().sum::<u64>(),
        40,
        "§19.3: the T deltas must sum to the final sim time (40), got {:?}",
        deltas
    );
    // The last record of the section is the closing T (nothing follows it).
    let last = body
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with("@section"))
        .next_back()
        .unwrap();
    assert_eq!(last, "T,+10", "§10.1: the trace must end at t=40, not t=30");
}

// ───────────────────────── selection & memories ─────────────────────────

/// §9.1/§9.2: an unpacked array (`logic [7:0] mem [0:3]`) contributes one
/// dictionary entry per element, under the module that declares it, and its
/// elements emit deltas like any other signal.
#[test]
fn memory_elements_are_dumped() {
    let xt = dump("mem", KITCHEN);
    for i in 0..4 {
        let s = sig_line(&xt, &format!("mem[{}]", i));
        assert!(s.contains(",u8,enc=delta,width=8"), "§9.2: {}", s);
    }
    assert_eq!(values_of(&xt, "mem[1]"), vec!["X", "0x0", "0xbe"]);
    assert_eq!(values_of(&xt, "mem[3]"), vec!["X", "0x0"]);
}

/// `--xtrace-scope`: only the signals under the scope reach the dictionary, and
/// the trace stays conformant — the alias canonicalization has to cope with the
/// backing net (`top.src_bus`) being filtered OUT of the dump, in which case the
/// first name inside the scope stands in for it.
#[test]
fn scope_filter_keeps_the_dump_conformant() {
    let xt = dump_scoped("scope", KITCHEN, &["u_mid".to_string()]);
    let names: Vec<&str> = xt
        .lines()
        .filter(|l| l.starts_with("S,"))
        .map(|l| l.split(',').nth(3).unwrap())
        .collect();
    assert_eq!(names, vec!["din", "dout", "din", "dout"], "scoped dictionary");
    assert!(!xt.contains(",src_bus,"), "out-of-scope signals stay out");
    // The two leaf names alias the two mid names (which now stand in as the
    // canonical ids), and only those canonical ids carry deltas.
    let canon = sig_lines_all(&xt, "din")[0].split(',').nth(1).unwrap().to_string();
    assert!(sig_lines_all(&xt, "din")[1].contains(&format!(",alias={}", canon)));
}
