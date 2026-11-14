//! Issue #1103: `$dumpvars` throws away an indexed argument — the only way to
//! name ONE generated instance — and dumps the whole design instead.
//!
//! `$dumpvars(0, tb_top.u_chan.g_lane[2].u_lane)` names a generate-block
//! instance. The parser does not keep that path in an `ExprKind::Ident` (it
//! builds `Index`/`MemberAccess` around the identifier), and the `$dumpvars`
//! handler matched only `ExprKind::Ident`, so the argument was discarded with
//! no diagnostic: the filter list came back empty, empty means "no filter",
//! and the run wrote every signal in the design. The same happened to one
//! element of an unpacked array (`tb_top.vec[3]`) and to the hand-written
//! enumeration of generated lanes, which is the workaround that ought to be
//! equivalent to the indexed form.
//!
//! §21.7.1.4 keeps two behaviors apart: `$dumpvars` with no scope argument
//! dumps the whole design, and `$dumpvars(0, scope)` dumps that scope and
//! everything below it — the bug collapsed the second into the first. §27.6.1
//! spells a generate block's scope with its index (`g_lane[2]`), which is the
//! spelling the argument has to survive into.
//!
//! `--fst-scope` takes the very same strings as raw CLI text and never had the
//! problem, so it is the oracle here: the two paths must select the same
//! signals.

use std::collections::BTreeSet;
use std::fs;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use fst_reader::{FstHierarchyEntry, FstReader};

fn xezim() -> String {
    // Cargo names the binary it built for the integration tests.
    env!("CARGO_BIN_EXE_xezim").to_string()
}

static SEQ: AtomicUsize = AtomicUsize::new(0);

/// Three generated lanes under `u_chan`, two generated probes under the TB,
/// and an unpacked array — the shapes a `$dumpvars` argument has to reach.
/// Every signal toggles, so none of them is dropped as a constant.
const SRC: &str = r#"
`timescale 1ns/1ps
module stage;
  logic [7:0] s = 0;
  logic en = 0;
  always #3 s <= s + 8'd1;
  always #5 en <= ~en;
endmodule

module probe;
  logic p0 = 0;
  always #8 p0 <= ~p0;
  genvar q;
  generate
    for (q = 0; q < 2; q++) begin : g_leaf
      stage u_stage();
    end
  endgenerate
endmodule

module lane;
  logic [7:0] s0 = 0, s1 = 0;
  logic ctrl = 0;
  always #2 s0 <= ~s0;
  always #4 s1 <= ~s1;
  always #7 ctrl <= ~ctrl;
endmodule

module chan;
  logic [7:0] own_a = 0, own_b = 0;
  logic [3:0] hist [0:3];
  initial hist[0] = 4'd0;
  always #6 own_a <= ~own_a;
  always #9 own_b <= ~own_b;
  always #11 hist[0] <= hist[0] + 4'd1;
  genvar g;
  generate
    for (g = 0; g < 3; g++) begin : g_lane
      lane u_lane();
    end
  endgenerate
endmodule

module tb_top;
  logic clk, rst_n, dbg;
  logic [31:0] vec [0:3];
  chan u_chan();
  genvar b;
  generate
    for (b = 0; b < 2; b++) begin : g_blk
      probe u_probe();
    end
  endgenerate
  initial clk = 1'b0;
  initial rst_n = 1'b0;
  always #5 clk = ~clk;
  always #13 dbg <= ~dbg;
  initial begin
    $dumpfile("{VCD}");
    $dumpvars{CALL};
    #60;
    $finish;
  end
endmodule
"#;

fn tmp(stem: &str, ext: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "xezim_dumpvars_1103_{}_{}_{}.{}",
        stem,
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed),
        ext
    ));
    let _ = fs::remove_file(&p);
    p
}

/// What one `$dumpvars` run produced.
struct Dump {
    /// Combined stdout+stderr — the `[VCD] dumping N signals (filter_scopes=…)`
    /// banner and the warnings are part of the contract, not just the file.
    out: String,
    /// Fully-qualified `$scope` paths, in the order the file declares them.
    scopes: Vec<String>,
    /// Fully-qualified `$var` names, sorted.
    vars: Vec<String>,
}

/// Read a dump's scope tree back into fully-qualified names: a `$var` line
/// carries only the leaf, so the enclosing `$scope` stack has to be joined on
/// (this is exactly how a viewer — and the issue's grader — names a signal).
fn read_vcd(path: &Path) -> (Vec<String>, Vec<String>) {
    let text = fs::read_to_string(path).unwrap_or_default();
    let mut stack: Vec<String> = Vec::new();
    let mut scopes = Vec::new();
    let mut vars = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("$scope") {
            let mut it = rest.split_whitespace();
            let _kind = it.next();
            if let Some(name) = it.next() {
                stack.push(name.to_string());
                scopes.push(stack.join("."));
            }
        } else if line.starts_with("$upscope") {
            stack.pop();
        } else if let Some(rest) = line.strip_prefix("$var") {
            let mut it = rest.split_whitespace();
            let _kind = it.next();
            let _width = it.next();
            let _code = it.next();
            if let Some(name) = it.next() {
                stack.push(name.to_string());
                vars.push(stack.join("."));
                stack.pop();
            }
        }
    }
    vars.sort();
    (scopes, vars)
}

/// Run `$dumpvars(DEPTH, ARGS)` and report what it wrote.
fn dump(tag: &str, depth: &str, args: &str) -> Dump {
    let dir = tmp(tag, "d");
    fs::create_dir_all(&dir).expect("mkdir");
    let vcd = dir.join("out.vcd");
    let sv = dir.join("dut.sv");
    let call = if args.trim().is_empty() {
        format!("({depth})")
    } else {
        format!("({depth}, {args})")
    };
    fs::write(
        &sv,
        SRC.replace("{VCD}", &vcd.display().to_string())
            .replace("{CALL}", &call),
    )
    .expect("write sv");

    let out = Command::new(xezim())
        .current_dir(&dir)
        .env("XEZIM_NO_CACHE", "1")
        .args(["--simulate", "--wave", "--max-time", "500", "-s", "tb_top"])
        .arg(&sv)
        .output()
        .expect("run xezim");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let (scopes, vars) = if vcd.is_file() {
        read_vcd(&vcd)
    } else {
        (Vec::new(), Vec::new())
    };
    Dump {
        out: text,
        scopes,
        vars,
    }
}

/// The same selection through the CLI: `--fst-scope` is given the very same
/// strings `$dumpvars` is given, as raw text that never touches the parser.
fn fst_vars(tag: &str, scopes: &[&str]) -> Vec<String> {
    let dir = tmp(tag, "f");
    fs::create_dir_all(&dir).expect("mkdir");
    let fst = dir.join("out.fst");
    let sv = dir.join("dut.sv");
    // No $dumpvars in the source: the FST is opened by --fst, and the scope
    // strings reach it only through the CLI.
    fs::write(
        &sv,
        SRC.replace("{VCD}", &dir.join("unused.vcd").display().to_string())
            .replace("$dumpvars{CALL};", ""),
    )
    .expect("write sv");

    let mut cmd = Command::new(xezim());
    cmd.current_dir(&dir)
        .env("XEZIM_NO_CACHE", "1")
        .args(["--simulate", "--max-time", "500", "-s", "tb_top"])
        .arg("--fst")
        .arg(&fst);
    for sc in scopes {
        cmd.arg("--fst-scope").arg(sc);
    }
    let out = cmd.arg(&sv).output().expect("run xezim");
    assert!(
        fst.is_file(),
        "--fst-scope {scopes:?} wrote no FST:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let file = fs::File::open(&fst).expect("open fst");
    let mut reader = FstReader::open_and_read_time_table(BufReader::new(file))
        .expect("FST header does not decode");
    let mut stack: Vec<String> = Vec::new();
    let mut vars = Vec::new();
    reader
        .read_hierarchy(|e| match e {
            FstHierarchyEntry::Scope { name, .. } => stack.push(name),
            FstHierarchyEntry::UpScope => {
                stack.pop();
            }
            FstHierarchyEntry::Var { name, .. } => {
                // FST carries the declared range inside the var name
                // (`s0 [7:0]`, `vec[0] [31:0]`); the reference is the first word.
                let mut full = stack.clone();
                full.push(name.split_whitespace().next().unwrap_or(&name).to_string());
                vars.push(full.join("."));
            }
            _ => {}
        })
        .expect("FST hierarchy does not decode");
    vars.sort();
    vars
}

/// Names in `vars` that are `prefix` itself or sit under it.
fn under(vars: &[String], prefix: &str) -> Vec<String> {
    let dot = format!("{prefix}.");
    vars.iter()
        .filter(|n| **n == prefix || n.starts_with(&dot))
        .cloned()
        .collect()
}

fn set(list: &[&str]) -> BTreeSet<String> {
    list.iter().map(|s| s.to_string()).collect()
}

// ---------------------------------------------------------------------------
// The issue: an indexed generate-block argument.
// ---------------------------------------------------------------------------

/// The headline case. `$dumpvars(0, tb_top.u_chan.g_lane[2].u_lane)` used to
/// select nothing, the empty filter then meant "no filter", and the run wrote
/// every signal in the design.
#[test]
fn an_indexed_generate_instance_dumps_that_subtree_and_nothing_else() {
    let d = dump("lane2", "0", "tb_top.u_chan.g_lane[2].u_lane");
    // The argument survived parsing: the banner reports the filter it was
    // given, not the empty list that meant "dump everything".
    assert!(
        d.out
            .contains(r#"filter_scopes=["tb_top.u_chan.g_lane[2].u_lane"]"#),
        "the indexed argument did not reach the filter list:\n{}",
        d.out
    );
    // The generated block scope and the instance under it are both in the tree.
    for want in ["tb_top.u_chan.g_lane[2]", "tb_top.u_chan.g_lane[2].u_lane"] {
        assert!(
            d.scopes.iter().any(|s| s == want),
            "no {want} $scope in {:?}",
            d.scopes
        );
    }
    // Exactly that one lane's signals — no sibling lane, no chan-level signal,
    // nothing from the TB.
    assert_eq!(
        d.vars,
        set(&[
            "tb_top.u_chan.g_lane[2].u_lane.ctrl",
            "tb_top.u_chan.g_lane[2].u_lane.s0",
            "tb_top.u_chan.g_lane[2].u_lane.s1",
        ])
        .into_iter()
        .collect::<Vec<_>>(),
        "an indexed $dumpvars argument did not select exactly one lane"
    );
}

/// Naming the generated BLOCK rather than the instance inside it selects the
/// whole generated scope — and still only that one.
#[test]
fn an_indexed_generate_block_dumps_its_own_branch() {
    let d = dump("block1", "0", "tb_top.u_chan.g_lane[1]");
    assert_eq!(
        d.vars,
        set(&[
            "tb_top.u_chan.g_lane[1].u_lane.ctrl",
            "tb_top.u_chan.g_lane[1].u_lane.s0",
            "tb_top.u_chan.g_lane[1].u_lane.s1",
        ])
        .into_iter()
        .collect::<Vec<_>>(),
        "an indexed $dumpvars of a generate block selected the wrong signals"
    );
    assert!(
        !d.scopes.iter().any(|s| s.contains("g_lane[0]")),
        "a sibling generate branch leaked in: {:?}\n{}",
        d.scopes,
        d.out
    );
}

/// Two indices in one path: a generated block inside a generated block.
#[test]
fn two_indices_in_one_path_select_one_nested_branch() {
    let d = dump("nested", "0", "tb_top.g_blk[1].u_probe.g_leaf[0].u_stage");
    assert_eq!(
        d.vars,
        set(&[
            "tb_top.g_blk[1].u_probe.g_leaf[0].u_stage.en",
            "tb_top.g_blk[1].u_probe.g_leaf[0].u_stage.s",
        ])
        .into_iter()
        .collect::<Vec<_>>(),
        "a doubly indexed $dumpvars argument selected the wrong signals"
    );
}

/// A generated instance hanging directly off the testbench top: the whole
/// subtree of that one branch, and not one signal of the sibling branch.
#[test]
fn a_generated_instance_under_the_top_is_selectable() {
    let d = dump("probe1", "0", "tb_top.g_blk[1].u_probe");
    assert_eq!(
        d.vars,
        set(&[
            "tb_top.g_blk[1].u_probe.p0",
            "tb_top.g_blk[1].u_probe.g_leaf[0].u_stage.en",
            "tb_top.g_blk[1].u_probe.g_leaf[0].u_stage.s",
            "tb_top.g_blk[1].u_probe.g_leaf[1].u_stage.en",
            "tb_top.g_blk[1].u_probe.g_leaf[1].u_stage.s",
        ])
        .into_iter()
        .collect::<Vec<_>>(),
        "the generated probe branch was not selected"
    );
    // The other branch of the same generate block must stay out.
    assert!(
        under(&d.vars, "tb_top.g_blk[0]").is_empty(),
        "g_blk[0] leaked into a $dumpvars of g_blk[1]: {:?}",
        d.vars
    );
}

/// The three generated lanes' instances, spelled by hand.
const ALL_LANES: &str = "tb_top.u_chan.g_lane[0].u_lane, \
                         tb_top.u_chan.g_lane[1].u_lane, \
                         tb_top.u_chan.g_lane[2].u_lane";

// ---------------------------------------------------------------------------
// Unpacked-array elements, and the workarounds that must agree.
// ---------------------------------------------------------------------------

/// `$dumpvars(0, tb_top.vec[3])` names ONE element of an unpacked array. It
/// must dump that element and nothing else — not the whole array, not the TB.
#[test]
fn one_element_of_an_unpacked_array_dumps_only_that_element() {
    let d = dump("vec3", "0", "tb_top.vec[3]");
    assert_eq!(
        d.vars,
        set(&["tb_top.vec[3]"]).into_iter().collect::<Vec<_>>(),
        "an indexed array element did not dump just that element"
    );
}

/// The workaround users fall back to — naming every generated lane by hand —
/// must be equivalent to the indexed form, and must not blow up into a
/// whole-design dump either.
#[test]
fn naming_every_generated_lane_by_hand_dumps_exactly_those_lanes() {
    let d = dump("all_lanes", "0", ALL_LANES);
    let mut want: Vec<String> = Vec::new();
    for lane in 0..3 {
        for sig in ["ctrl", "s0", "s1"] {
            want.push(format!("tb_top.u_chan.g_lane[{lane}].u_lane.{sig}"));
        }
    }
    want.sort();
    assert_eq!(
        d.vars, want,
        "the hand-written lane enumeration selected the wrong signals"
    );
}

/// Mixing an indexed scope argument with a plain one: both must survive. The
/// old handler kept neither the first (not an `Ident`) nor anything after it
/// once the filter list had been filled by luck.
#[test]
fn an_indexed_scope_and_a_plain_variable_are_both_honoured() {
    let d = dump("mixed", "0", "tb_top.u_chan.g_lane[1].u_lane, tb_top.dbg");
    assert_eq!(
        d.vars,
        set(&[
            "tb_top.dbg",
            "tb_top.u_chan.g_lane[1].u_lane.ctrl",
            "tb_top.u_chan.g_lane[1].u_lane.s0",
            "tb_top.u_chan.g_lane[1].u_lane.s1",
        ])
        .into_iter()
        .collect::<Vec<_>>(),
        "mixing an indexed scope with a plain name lost one of them"
    );
}

/// An array element and a generated scope in the same call.
#[test]
fn an_array_element_and_a_generated_scope_are_both_honoured() {
    let d = dump("mixed_arr", "0", "tb_top.vec[0], tb_top.g_blk[0].u_probe");
    assert_eq!(
        d.vars,
        set(&[
            "tb_top.g_blk[0].u_probe.p0",
            "tb_top.g_blk[0].u_probe.g_leaf[0].u_stage.en",
            "tb_top.g_blk[0].u_probe.g_leaf[0].u_stage.s",
            "tb_top.g_blk[0].u_probe.g_leaf[1].u_stage.en",
            "tb_top.g_blk[0].u_probe.g_leaf[1].u_stage.s",
            "tb_top.vec[0]",
        ])
        .into_iter()
        .collect::<Vec<_>>(),
        "an array element and a scope did not combine"
    );
}

// ---------------------------------------------------------------------------
// The silent drop itself: an argument that is not a name must be reported,
// and must not turn into a whole-design dump.
// ---------------------------------------------------------------------------

/// An argument that is not a name at all used to vanish without a word.
#[test]
fn an_argument_that_is_not_a_name_is_reported() {
    let d = dump("bogus", "0", "tb_top.dbg + 1");
    assert!(
        d.out.contains("not a scope or variable name"),
        "a $dumpvars argument the tool cannot name was dropped silently:\n{}",
        d.out
    );
}

/// The fail-open half of the bug: with no usable argument the run must not
/// fall back to dumping the entire design, because an empty filter means
/// "no filter".
#[test]
fn no_usable_argument_selects_nothing_rather_than_everything() {
    let d = dump("bogus_only", "0", "1 + 1");
    assert!(
        d.vars.is_empty(),
        "a $dumpvars whose arguments were all rejected dumped {:#?}:\n{}",
        d.vars,
        d.out
    );
    assert!(
        d.out.contains("selects nothing"),
        "the fail-closed decision was not reported:\n{}",
        d.out
    );
}

/// One bad argument among good ones is reported but does not sink the rest.
#[test]
fn a_bad_argument_among_good_ones_is_reported_but_survivable() {
    let d = dump("mixed_bad", "0", "tb_top.dbg, 1 + 1");
    assert!(
        d.out.contains("not a scope or variable name"),
        "the bad argument was not reported:\n{}",
        d.out
    );
    assert_eq!(
        d.vars,
        set(&["tb_top.dbg"]).into_iter().collect::<Vec<_>>(),
        "the good argument was lost along with the bad one"
    );
}

// ---------------------------------------------------------------------------
// `--fst-scope` is the oracle: the same strings must select the same signals.
// ---------------------------------------------------------------------------

/// FST var names carry their declared range (`s0 [7:0]`), which `fst_vars`
/// drops; both writers root their tree at the top module, so a signal is
/// spelled the same fully-qualified way on either side and the two sets can be
/// compared name for name.
fn fst_names(tag: &str, scopes: &[&str]) -> BTreeSet<String> {
    fst_vars(tag, scopes).into_iter().collect()
}

fn vcd_names(vars: &[String]) -> BTreeSet<String> {
    vars.iter().cloned().collect()
}

/// `$dumpvars(0, X)` and `--fst-scope X` are the same request in two
/// spellings; one of them (the CLI) never had this bug.
#[test]
fn dumpvars_and_fst_scope_select_the_same_signals() {
    for arg in [
        "tb_top.u_chan.g_lane[2].u_lane",
        "tb_top.u_chan.g_lane[1]",
        "tb_top.g_blk[1].u_probe",
        "tb_top.g_blk[0].u_probe.g_leaf[1].u_stage",
        "tb_top.vec[2]",
        "tb_top.u_chan",
        "tb_top.dbg",
    ] {
        let d = dump("parity_vcd", "0", arg);
        let f = fst_names("parity_fst", &[arg]);
        assert_eq!(
            vcd_names(&d.vars),
            f,
            "$dumpvars(0, {arg}) and --fst-scope {arg} selected different signals\n{}",
            d.out
        );
        // The VCD must not have fallen back to the whole design either.
        assert!(
            !d.vars.is_empty(),
            "$dumpvars(0, {arg}) selected nothing\n{}",
            d.out
        );
    }
}

// ---------------------------------------------------------------------------
// Controls: the forms that already worked must keep working.
// ---------------------------------------------------------------------------

/// A plain scope still dumps its whole subtree.
#[test]
fn a_plain_scope_still_dumps_its_subtree() {
    let d = dump("plain_scope", "0", "tb_top.u_chan");
    assert!(
        under(&d.vars, "tb_top.u_chan.own_a")
            .iter()
            .next()
            .is_some(),
        "chan-level signal lost: {:?}",
        d.vars
    );
    assert!(
        d.vars.iter().any(|n| n == "tb_top.u_chan.hist[0]"),
        "{:?}",
        d.vars
    );
    for lane in 0..3 {
        assert!(
            !under(&d.vars, &format!("tb_top.u_chan.g_lane[{lane}]")).is_empty(),
            "generated lane {lane} missing from a $dumpvars of its parent: {:?}",
            d.vars
        );
    }
    assert!(
        under(&d.vars, "tb_top.dbg").is_empty() && under(&d.vars, "tb_top.g_blk").is_empty(),
        "a $dumpvars of u_chan reached outside it: {:?}",
        d.vars
    );
}

/// A plain variable still dumps just that variable.
#[test]
fn a_plain_variable_still_dumps_just_it() {
    let d = dump("plain_var", "0", "tb_top.dbg");
    assert_eq!(
        d.vars,
        set(&["tb_top.dbg"]).into_iter().collect::<Vec<_>>(),
        "a plain $dumpvars variable changed behaviour"
    );
}

/// With no argument at all the whole design is still dumped — the case the
/// fail-closed rule above must not touch.
#[test]
fn no_argument_still_dumps_the_whole_design() {
    let d = dump("everything", "0", "");
    for want in [
        "tb_top.dbg",
        "tb_top.vec[3]",
        "tb_top.u_chan.g_lane[2].u_lane.s0",
        "tb_top.g_blk[1].u_probe.g_leaf[0].u_stage.s",
    ] {
        assert!(
            d.vars.iter().any(|n| n == want),
            "{want} missing from a whole-design dump of {:#?}\n{}",
            d.vars,
            d.out
        );
    }
}

/// Naming the top module is the same request as naming nothing.
#[test]
fn naming_the_top_module_still_dumps_the_whole_design() {
    let d = dump("top_arg", "0", "tb_top");
    assert!(
        d.vars
            .iter()
            .any(|n| n == "tb_top.u_chan.g_lane[0].u_lane.s0")
            && d.vars.iter().any(|n| n == "tb_top.dbg"),
        "a $dumpvars of the top module no longer dumps everything: {:?}",
        d.vars
    );
}
