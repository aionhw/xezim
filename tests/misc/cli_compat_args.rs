//! Other simulators' command-line spellings (src/cli_compat.rs): each one runs
//! a small design and must give the same result as the equivalent native
//! xezim flags. The `-do` subset parser is a pure function, tested directly by
//! including the binary's module source.

use std::path::{Path, PathBuf};
use std::process::Command;

#[allow(dead_code)]
#[path = "../../src/cli_compat.rs"]
mod cli_compat;

use cli_compat::{DoPlan, DoRun, bare_top_name, c_takes_file, plan_do_scripts};

fn xezim() -> String {
    let mut p = std::env::current_exe().expect("current_exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("xezim").to_string_lossy().into_owned()
}

/// A fresh scratch directory per test (tests run in parallel).
fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("xezim_cli_compat_{}_{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Run xezim in `dir`; returns (exit code, stdout, stderr).
fn run_in(dir: &Path, args: &[&str]) -> (i32, String, String) {
    let out = Command::new(xezim())
        .current_dir(dir)
        .args(args)
        .output()
        .expect("run xezim");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// Stdout of a run that must succeed.
fn ok_stdout(dir: &Path, args: &[&str]) -> String {
    let (code, out, err) = run_in(dir, args);
    assert_eq!(code, 0, "xezim {:?} failed:\n{}{}", args, out, err);
    out
}

fn plan(text: &str) -> Result<DoPlan, String> {
    plan_do_scripts(&[(text.to_string(), "-do".to_string())])
}

fn script(text: &str) -> Result<DoRun, String> {
    plan(text).map(|p| p.run)
}

#[test]
fn do_subset_plans() {
    assert_eq!(script("run -all; quit -f"), Ok(DoRun::All));
    assert_eq!(script("run 100ns; quit"), Ok(DoRun::For(100)));
    assert_eq!(script("run 1us\nrun 500 ns\nexit"), Ok(DoRun::For(1500)));
    assert_eq!(script("run 2ms"), Ok(DoRun::For(2_000_000)));
    assert_eq!(script("run 3000ps; quit -force"), Ok(DoRun::For(3)));
    assert_eq!(script("run 1sec"), Ok(DoRun::For(1_000_000_000)));
    assert_eq!(script("run 10ns; run -all; run 5ns"), Ok(DoRun::All));
    // Everything after a quit is never reached.
    assert_eq!(script("run 10ns; quit; run -all"), Ok(DoRun::For(10)));
    assert_eq!(script("quit -f"), Ok(DoRun::Load));
    assert_eq!(
        script("# setup\nrun 20ns ;# twenty\n\nquit"),
        Ok(DoRun::For(20))
    );
}

/// The AVIP makefiles' script: waveform logging and coverage saves are
/// accepted with one warning per kind of command and change nothing else.
#[test]
fn do_subset_ignores_logging_and_coverage() {
    let p = plan(
        "log -r /*; add wave -r /*; coverage save -onexit -assert -directive -cvg \
         -codeAll t/t_coverage.ucdb; run -all; exit",
    )
    .unwrap();
    assert_eq!(p.run, DoRun::All);
    assert_eq!(p.warnings.len(), 3, "{:?}", p.warnings);
    assert!(
        p.warnings[0].contains("`log -r /*` is ignored"),
        "{:?}",
        p.warnings
    );
    assert!(
        p.warnings[1].contains("`add wave -r /*`"),
        "{:?}",
        p.warnings
    );
    assert!(
        p.warnings[2].contains("`coverage save -onexit"),
        "{:?}",
        p.warnings
    );
    // xezim does write coverage, to its own file.
    assert!(p.warnings[2].contains("xezim_cov.json"), "{:?}", p.warnings);
    // One warning per kind, however often it appears.
    let p = plan("log a; log -r /*\nadd wave x; add wave y; coverage report -file c.txt; run 5ns")
        .unwrap();
    assert_eq!(p.run, DoRun::For(5));
    assert_eq!(p.warnings.len(), 3, "{:?}", p.warnings);
    assert!(plan("run -all").unwrap().warnings.is_empty());
}

#[test]
fn do_subset_rejects_everything_else() {
    for bad in [
        "add list -r /*",
        "add",
        "coverage exclude -du tb",
        "coverage",
        "run",
        "run 100",
        "run 1.5ps",
        "run 10 parsecs",
        "logfile x; run -all",
        "quit -code 3",
        "onfinish stop",
    ] {
        assert!(script(bad).is_err(), "`{}` must be rejected", bad);
    }
    let e = script("run 10ns; vcd file x.vcd").unwrap_err();
    assert!(
        e.contains("vcd file x.vcd") && e.contains("run -all"),
        "{}",
        e
    );
}

#[test]
fn bare_names_and_dash_c() {
    let libs = vec!["mylib".to_string()];
    assert_eq!(bare_top_name("tb_top", &libs).as_deref(), Some("tb_top"));
    assert_eq!(
        bare_top_name("work.tb_top", &libs).as_deref(),
        Some("tb_top")
    );
    assert_eq!(bare_top_name("mylib.hvl", &libs).as_deref(), Some("hvl"));
    assert_eq!(bare_top_name("other.hvl", &libs), None);
    assert_eq!(bare_top_name("missing.sv", &libs), None);
    assert_eq!(bare_top_name("dir/tb", &libs), None);
    // `-c` keeps reading an args file whenever one could follow.
    assert!(c_takes_file(Some("files.f"), &libs));
    assert!(c_takes_file(Some("sub/files"), &libs));
    assert!(!c_takes_file(Some("-quiet"), &libs));
    assert!(!c_takes_file(Some("+UVM_TESTNAME=t"), &libs));
    assert!(!c_takes_file(Some("tb_top"), &libs));
    assert!(!c_takes_file(Some("work.tb_top"), &libs));
    assert!(!c_takes_file(None, &libs));
}

const TIMED: &str = "\
module tb;
  initial begin
    #5 $display(\"A t=%0t\", $time);
    #95 $display(\"B t=%0t\", $time);
    #1 $display(\"C t=%0t\", $time);
    #199899 $display(\"D t=%0t\", $time);
    $finish;
  end
endmodule
";

const LATE_FINISH: &str = "\
module tb;
  initial begin
    #150000000 $display(\"L t=%0t\", $time);
    $finish;
  end
endmodule
";

#[test]
fn do_run_time_matches_max_time() {
    let d = scratch("dorun");
    std::fs::write(d.join("tb.sv"), TIMED).unwrap();
    let native = ok_stdout(&d, &["tb.sv", "--max-time", "100"]);
    assert!(
        native.contains("B t=100") && !native.contains("C t="),
        "{}",
        native
    );
    assert_eq!(ok_stdout(&d, &["tb.sv", "-do", "run 100ns; quit"]), native);
    assert_eq!(
        ok_stdout(&d, &["tb.sv", "-do", "run 60 ns\nrun 40ns"]),
        native
    );
    std::fs::write(d.join("run.do"), "# stimulus\nrun 100ns\nquit -f\n").unwrap();
    assert_eq!(ok_stdout(&d, &["tb.sv", "-do", "run.do"]), native);
    assert_eq!(ok_stdout(&d, &["tb.sv", "-do", "do run.do"]), native);
    // --max-time stays a hard cap over the script.
    assert_eq!(
        ok_stdout(&d, &["tb.sv", "--max-time", "100", "-do", "run -all"]),
        native
    );
}

#[test]
fn do_run_all_lifts_the_default_cap() {
    let d = scratch("doall");
    std::fs::write(d.join("tb.sv"), TIMED).unwrap();
    // $finish is at 200 us, inside the default 100 ms cap.
    let native = ok_stdout(&d, &["tb.sv", "--max-time", "1ms"]);
    assert!(native.contains("D t=200000") && native.contains("$finish called"));
    assert_eq!(ok_stdout(&d, &["tb.sv"]), native);
    assert_eq!(
        ok_stdout(&d, &["tb.sv", "-do", "run -all; quit -f"]),
        native
    );
    // $finish at 150 ms, past the default cap: only `run -all` reaches it.
    let late = scratch("doall_late");
    std::fs::write(late.join("tb.sv"), LATE_FINISH).unwrap();
    let capped = ok_stdout(&late, &["tb.sv"]);
    assert!(!capped.contains("L t="), "{}", capped);
    let all = ok_stdout(&late, &["tb.sv", "-do", "run -all; quit -f"]);
    assert!(
        all.contains("L t=150000000") && all.contains("$finish called"),
        "{}",
        all
    );
    // Quitting before any run only elaborates.
    let (code, out, _) = run_in(&d, &["tb.sv", "-do", "quit -f"]);
    assert_eq!(code, 0);
    assert!(
        out.contains("Elaboration successful") && !out.contains("A t="),
        "{}",
        out
    );
    // An unknown command stops the run before it starts.
    let (code, _, err) = run_in(&d, &["tb.sv", "-do", "add list -r /*; run -all"]);
    assert_eq!(code, 1);
    assert!(
        err.contains("unsupported command `add list -r /*`"),
        "{}",
        err
    );
    // Logging and coverage commands are warned about once and skipped.
    let (code, out, err) = run_in(
        &d,
        &[
            "tb.sv",
            "-do",
            "log -r /*; add wave -r /*; coverage save -onexit c.ucdb; run -all; exit",
        ],
    );
    assert_eq!(code, 0, "{}", err);
    assert_eq!(out, native);
    assert_eq!(err.matches("Warning: -do: `").count(), 3, "{}", err);
}

const STOPS: &str = "\
module tb;
  initial begin
    #5 $display(\"A t=%0t\", $time);
    #200 $display(\"B t=%0t\", $time);
  end
  final $display(\"final t=%0t\", $time);
endmodule
";

/// After `run <time>` the current time is the end of the run, not the last
/// event before it: `final` blocks and the closing line report it, also when
/// nothing is scheduled any more (cross-checked on the reference simulator).
#[test]
fn do_run_time_ends_at_the_stop_time() {
    let d = scratch("stops");
    std::fs::write(d.join("tb.sv"), STOPS).unwrap();
    let out = ok_stdout(&d, &["tb.sv", "-do", "run 100ns; quit -f"]);
    assert!(
        out.contains("A t=5\nfinal t=100\nSimulation finished at time 100\n"),
        "{}",
        out
    );
    let out = ok_stdout(&d, &["tb.sv", "-do", "run 100ns; run 23ns; quit -f"]);
    assert!(
        out.contains("final t=123\nSimulation finished at time 123\n"),
        "{}",
        out
    );
    let out = ok_stdout(&d, &["tb.sv", "-do", "run -all"]);
    assert!(out.contains("B t=205\nfinal t=205\n"), "{}", out);
    std::fs::write(
        d.join("idle.sv"),
        "`timescale 1ns/1ps\nmodule tb; initial #5 $display(\"A t=%0t\", $time);\n\
         final $display(\"final t=%0t\", $time); endmodule\n",
    )
    .unwrap();
    let out = ok_stdout(&d, &["idle.sv", "-do", "run 100ns; quit -f"]);
    assert!(out.contains("A t=5000\nfinal t=100000\n"), "{}", out);
}

#[test]
fn defines_incdirs_and_args_files() {
    let d = scratch("defs");
    for (dir, file, text) in [
        ("inc1", "one.svh", "`define ONE 11\n"),
        ("inc2", "two.svh", "`define TWO 22\n"),
    ] {
        std::fs::create_dir_all(d.join(dir)).unwrap();
        std::fs::write(d.join(dir).join(file), text).unwrap();
    }
    std::fs::write(
        d.join("tb.sv"),
        "`include \"one.svh\"\n`include \"two.svh\"\n\
         module tb; initial begin\n\
         `ifdef FLAG $display(\"FLAG set\"); `endif\n\
         $display(\"VAL=%0d ONE=%0d TWO=%0d\", `VAL, `ONE, `TWO);\n\
         end endmodule\n",
    )
    .unwrap();
    let native = ok_stdout(
        &d,
        &[
            "-D", "FLAG", "-D", "VAL=7", "-I", "inc1", "-I", "inc2", "tb.sv",
        ],
    );
    assert!(native.contains("FLAG set") && native.contains("VAL=7 ONE=11 TWO=22"));
    let compat = ok_stdout(
        &d,
        &["-sv", "+define+FLAG+VAL=7", "+incdir+inc1+inc2", "tb.sv"],
    );
    assert_eq!(compat, native);
    // `-F`: +incdir+ paths inside resolve against the args file's directory.
    std::fs::create_dir_all(d.join("run")).unwrap();
    std::fs::write(
        d.join("files.f"),
        "+define+FLAG+VAL=7\n+incdir+inc1+inc2\n-sv -mfcu -work work\ntb.sv\n",
    )
    .unwrap();
    assert_eq!(ok_stdout(&d.join("run"), &["-F", "../files.f"]), native);
    assert_eq!(ok_stdout(&d, &["-file", "files.f"]), native);
}

/// `+incdir+.` in a `-F` file elsewhere is the file's own directory. `.`
/// always exists relative to the working directory too, which used to win,
/// so every include below the file's directory was lost.
#[test]
fn dash_f_incdir_dot_is_the_args_files_directory() {
    let d = scratch("incdir_dot");
    std::fs::create_dir_all(d.join("dv/inc")).unwrap();
    std::fs::create_dir_all(d.join("dv/tb")).unwrap();
    std::fs::write(d.join("dv/inc/val.svh"), "`define VAL 42\n").unwrap();
    std::fs::write(
        d.join("dv/tb/tb.sv"),
        "`include \"inc/val.svh\"\nmodule tb; initial $display(\"VAL=%0d\", `VAL); endmodule\n",
    )
    .unwrap();
    std::fs::write(d.join("dv/files.f"), "+incdir+.\ntb/tb.sv\n").unwrap();
    let out = ok_stdout(&d, &["-F", "dv/files.f"]);
    assert!(out.contains("VAL=42"), "{}", out);
}

const TOPS: &str = "\
module tb #(parameter int W = 4, parameter string S = \"def\") ();
  sub u_dflt();
  sub #(.W(3)) u_expl();
  initial $display(\"%m W=%0d hello=%0d bits=%0d\", W, S == \"hello\", $bits(logic [W-1:0]));
endmodule
module sub #(parameter int W = 1) ();
  initial $display(\"%m W=%0d\", W);
endmodule
module other;
  initial $display(\"%m other\");
endmodule
";

#[test]
fn bare_top_names_select_tops() {
    let d = scratch("tops");
    std::fs::write(d.join("t.sv"), TOPS).unwrap();
    let one = ok_stdout(&d, &["t.sv", "-s", "tb"]);
    assert!(one.contains("tb W=4") && !one.contains("other"), "{}", one);
    assert_eq!(ok_stdout(&d, &["t.sv", "tb"]), one);
    assert_eq!(
        ok_stdout(&d, &["-c", "-quiet", "-lib", "work", "t.sv", "work.tb"]),
        one
    );
    let two = ok_stdout(&d, &["t.sv", "-s", "tb", "-s", "other"]);
    assert!(two.contains("other") && two.contains("W=4"), "{}", two);
    assert_eq!(ok_stdout(&d, &["t.sv", "tb", "other"]), two);
    // `-c <args file>` keeps xezim's meaning.
    std::fs::write(d.join("files.f"), "t.sv\n").unwrap();
    assert_eq!(ok_stdout(&d, &["-c", "files.f", "-s", "tb"]), one);
}

fn filelist_top_fixture(tag: &str) -> PathBuf {
    let d = scratch(tag);
    std::fs::write(
        d.join("roots.sv"),
        r#"
module excluded_root;
  initial begin $display("ROOT|excluded"); #1 $finish; end
endmodule
module chosen_root;
  initial begin $display("ROOT|chosen"); #1 $finish; end
endmodule
module extra_root;
  initial begin $display("ROOT|extra"); #1 $finish; end
endmodule
"#,
    )
    .unwrap();
    d
}

fn assert_filelist_tops(dir: &Path, args: &[&str], expected: &[&str]) {
    let (code, out, err) = run_in(dir, args);
    assert_eq!(code, 0, "args={args:?}\n{out}{err}");
    assert!(!err.contains("ignored unrecognized option"), "{err}");
    let mut roots: Vec<_> = out
        .lines()
        .filter_map(|line| line.strip_prefix("ROOT|"))
        .collect();
    roots.sort_unstable();
    let mut expected = expected.to_vec();
    expected.sort_unstable();
    assert_eq!(roots, expected, "args={args:?}\n{out}{err}");
}

#[test]
fn filelist_top_cli_control() {
    let d = filelist_top_fixture("filelist_top_cli");
    std::fs::write(d.join("files.f"), "roots.sv\n").unwrap();
    assert_filelist_tops(&d, &["-f", "files.f", "-s", "chosen_root"], &["chosen"]);
    assert_filelist_tops(&d, &["-schosen_root", "-ffiles.f"], &["chosen"]);
}

#[test]
fn filelist_top_separate_argument() {
    let d = filelist_top_fixture("filelist_top_separate");
    std::fs::write(d.join("files.f"), "-s chosen_root\nroots.sv\n").unwrap();
    for flag in ["-f", "-F", "-c", "-file"] {
        assert_filelist_tops(&d, &[flag, "files.f"], &["chosen"]);
    }
    assert_filelist_tops(&d, &["-ffiles.f"], &["chosen"]);
}

#[test]
fn filelist_top_attached_argument() {
    let d = filelist_top_fixture("filelist_top_attached");
    std::fs::write(d.join("files.f"), "roots.sv\n-schosen_root\n").unwrap();
    for flag in ["-f", "-F", "-c", "-file"] {
        assert_filelist_tops(&d, &[flag, "files.f"], &["chosen"]);
    }
}

#[test]
fn filelist_top_nested_selection() {
    let d = filelist_top_fixture("filelist_top_nested");
    std::fs::create_dir(d.join("nested")).unwrap();
    for selection in ["-s chosen_root", "-schosen_root"] {
        std::fs::write(
            d.join("nested/inner.f"),
            format!("{selection}\n../roots.sv\n"),
        )
        .unwrap();
        for include in ["-f nested/inner.f", "-fnested/inner.f", "-F nested/inner.f"] {
            std::fs::write(d.join("outer.f"), format!("{include}\n")).unwrap();
            assert_filelist_tops(&d, &["-f", "outer.f"], &["chosen"]);
        }
    }
}

#[test]
fn filelist_top_accumulates_with_cli_and_preserves_seed_flags() {
    let d = filelist_top_fixture("filelist_top_multiple");
    std::fs::write(
        d.join("files.f"),
        "-sv -svseed 19 -seed=23\n-s chosen_root\nroots.sv\n",
    )
    .unwrap();
    assert_filelist_tops(
        &d,
        &["-s", "extra_root", "-f", "files.f"],
        &["chosen", "extra"],
    );
    assert_filelist_tops(&d, &["-f", "files.f", "-sextra_root"], &["chosen", "extra"]);
    std::fs::write(d.join("extra.f"), "-sextra_root\n").unwrap();
    std::fs::write(d.join("outer.f"), "-f files.f\n-f extra.f\n").unwrap();
    assert_filelist_tops(&d, &["-f", "outer.f"], &["chosen", "extra"]);
}

#[test]
fn filelist_top_missing_argument_is_an_error() {
    let d = filelist_top_fixture("filelist_top_missing");
    for selection in ["-s", "-s -sv", "-s \"\""] {
        std::fs::write(d.join("files.f"), format!("roots.sv\n{selection}\n")).unwrap();
        let (code, out, err) = run_in(&d, &["-f", "files.f"]);
        assert_ne!(code, 0, "{selection}: {out}{err}");
        assert!(
            err.contains("-s requires a top module") && err.contains("files.f"),
            "{err}"
        );
        assert!(!out.contains("ROOT|"), "{out}");
    }
}

#[test]
fn g_overrides_parameter_defaults() {
    let d = scratch("gpar");
    std::fs::write(d.join("t.sv"), TOPS).unwrap();
    let out = ok_stdout(&d, &["t.sv", "tb", "-gW=8", "-gS=hello"]);
    assert!(out.contains("tb W=8 hello=1 bits=8"), "{}", out);
    // Every defaulted instance takes it; an explicit instance value wins.
    assert!(
        out.contains("tb.u_dflt W=8") && out.contains("tb.u_expl W=3"),
        "{}",
        out
    );
    let quoted = ok_stdout(&d, &["t.sv", "tb", "-gW=8", "-gS=\"hello\""]);
    assert_eq!(quoted, out);
    // `-G` also beats a value given at the instantiation.
    let forced = ok_stdout(&d, &["t.sv", "tb", "-GW=8"]);
    assert!(
        forced.contains("tb W=8") && forced.contains("tb.u_dflt W=8"),
        "{}",
        forced
    );
    assert!(forced.contains("tb.u_expl W=8"), "{}", forced);
    // A path limits it to one module.
    let top_only = ok_stdout(&d, &["t.sv", "tb", "-g/tb/W=6"]);
    assert!(
        top_only.contains("tb W=6") && top_only.contains("tb.u_dflt W=1"),
        "{}",
        top_only
    );
    let (code, _, err) = run_in(&d, &["t.sv", "tb", "-gNOPE=1"]);
    assert_eq!(code, 0);
    assert!(err.contains("'NOPE'; ignored"), "{}", err);
}

const EXPRS: &str = "\
module tb #(parameter [5:0] P = 0, parameter logic [3:0][2:0] R = 0, parameter int N = 0);
  initial $display(\"P=%0d R=%h N=%0d\", P, R, N);
endmodule
";

/// A `-g` value is a constant expression (§6.20.2): a concatenation or
/// replication of sized literals (§11.4.12) sets a packed parameter. The
/// cocotb runner (contrib/cocotb) passes `parameters` values to `-g` as given,
/// and cocotb testbenches pass such values for packed parameters.
#[test]
fn g_takes_constant_expressions() {
    let d = scratch("gexpr");
    std::fs::write(d.join("t.sv"), EXPRS).unwrap();
    let args = ["t.sv", "-gP={3'd7,3'd5}", "-gR={4{3'd5}}", "-gN=8'hff - 1"];
    let out = ok_stdout(&d, &args);
    assert!(out.contains("P=61 R=b6d N=254"), "{}", out);
}

const LOCALS: &str = "\
module tb #(parameter int TOPP = 1) ();
  parameter int BODYP = 2;
  if (1) begin : g
    parameter int GP = 3;
    initial $display(\"GP=%0d\", GP);
  end
  for (genvar i = 0; i < 2; i++) begin : fl
    parameter int FP = 10;
    initial $display(\"FP=%0d\", FP);
  end
  bare u();
  initial $display(\"TOPP=%0d BODYP=%0d\", TOPP, BODYP);
endmodule
module bare;
  parameter int NP = 1;
  initial $display(\"NP=%0d\", NP);
endmodule
";

/// §6.20.1: a `parameter` inside a generate block, or in the body of a module
/// that has a parameter port list, is local. Neither `-g` nor `-G` reaches it:
/// the reference simulator warns that it is not found and runs on the
/// declared value. A body `parameter` of a module without a port list is
/// overridable.
#[test]
fn g_leaves_local_parameters_alone() {
    let d = scratch("glocal");
    std::fs::write(d.join("t.sv"), LOCALS).unwrap();
    let plain = ok_stdout(&d, &["t.sv"]);
    assert!(
        plain.contains("GP=3") && plain.contains("FP=10") && plain.contains("BODYP=2"),
        "{}",
        plain
    );
    for (flag, name) in [
        ("-gGP=5", "GP"),
        ("-GGP=5", "GP"),
        ("-gFP=11", "FP"),
        ("-gBODYP=12", "BODYP"),
        ("-GBODYP=12", "BODYP"),
    ] {
        let (code, out, err) = run_in(&d, &["t.sv", flag]);
        assert_eq!(code, 0, "{}", err);
        assert_eq!(out, plain, "{}", flag);
        assert!(
            err.contains(&format!("'{}'; ignored", name)),
            "{}: {}",
            flag,
            err
        );
    }
    let out = ok_stdout(&d, &["t.sv", "-gNP=7"]);
    assert!(out.contains("NP=7") && out.contains("TOPP=1"), "{}", out);
}

const RANDOM: &str = "\
module tb;
  int unsigned r;
  initial begin
    repeat (4) begin r = $urandom; $display(\"r=%0d\", r); end
  end
endmodule
";

#[test]
fn sv_seed_matches_plus_seed() {
    let d = scratch("seed");
    std::fs::write(d.join("tb.sv"), RANDOM).unwrap();
    let native = ok_stdout(&d, &["tb.sv", "+seed=42"]);
    assert_eq!(ok_stdout(&d, &["tb.sv", "-sv_seed", "42"]), native);
    assert_eq!(ok_stdout(&d, &["tb.sv", "-sv_seed", "42"]), native);
    assert_ne!(ok_stdout(&d, &["tb.sv", "-sv_seed", "7"]), native);
    let (code, out, err) = run_in(&d, &["tb.sv", "-sv_seed", "random"]);
    assert_eq!(code, 0, "{}{}", out, err);
    assert!(out.contains("[xezim] random seed:"), "{}", out);
    let (code, _, err) = run_in(&d, &["tb.sv", "-sv_seed", "abc"]);
    assert_eq!(code, 1);
    assert!(err.contains("-sv_seed"), "{}", err);
}

/// Every seed spelling, on the command line and in an args file, runs as
/// `+seed=<n>` does, and the run prints the seed it uses on stdout.
#[test]
fn seed_spellings_match_plus_seed() {
    let d = scratch("seed_spellings");
    std::fs::write(d.join("tb.sv"), RANDOM).unwrap();
    let native = ok_stdout(&d, &["tb.sv", "+seed=42"]);
    assert!(native.contains("[xezim] seed: 42\n"), "{}", native);
    for spelling in [
        &["-sv_seed", "42"][..],
        &["-svseed", "42"],
        &["-seed", "42"],
        &["-sv_seed=42"],
        &["-svseed=42"],
        &["-seed=42"],
    ] {
        let mut args = vec!["tb.sv"];
        args.extend_from_slice(spelling);
        assert_eq!(ok_stdout(&d, &args), native, "{:?}", spelling);
        let file = format!("tb.sv\n{}\n", spelling.join(" "));
        std::fs::write(d.join("seed.f"), file).unwrap();
        let (code, out, err) = run_in(&d, &["-f", "seed.f"]);
        assert_eq!(code, 0, "{:?} in an args file: {}", spelling, err);
        assert!(!err.contains("ignored unrecognized option"), "{}", err);
        assert_eq!(out, native, "{:?} in an args file", spelling);
    }
    // The last seed given is the run's, and only it is printed.
    assert_eq!(
        ok_stdout(&d, &["tb.sv", "-svseed", "7", "+seed=42"]),
        native
    );
    for (bad, flag) in [
        (&["-svseed", "x1"][..], "-svseed"),
        (&["-seed=-3"], "-seed"),
    ] {
        let mut args = vec!["tb.sv"];
        args.extend_from_slice(bad);
        let (code, out, err) = run_in(&d, &args);
        assert_eq!(code, 1, "{:?}: {}{}", bad, out, err);
        assert!(
            err.contains(&format!("{} wants a non-negative integer", flag)),
            "{:?}: {}",
            bad,
            err
        );
    }
    // No seed, no seed line.
    let unseeded = ok_stdout(&d, &["tb.sv"]);
    assert!(!unseeded.contains("seed"), "{}", unseeded);
}

/// The seed line is in the `-l` log, and a random seed read from the log
/// replays the run.
#[test]
fn seed_is_printed_to_the_log() {
    let d = scratch("seed_log");
    std::fs::write(d.join("tb.sv"), RANDOM).unwrap();
    let (code, out, err) = run_in(&d, &["tb.sv", "-sv_seed", "5", "-l", "run.log"]);
    assert_eq!(code, 0, "{}", err);
    assert_eq!(out, "");
    let log = std::fs::read_to_string(d.join("run.log")).unwrap();
    assert!(log.contains("[xezim] seed: 5\n"), "{}", log);
    let (code, _, err) = run_in(&d, &["tb.sv", "-sv_seed", "random", "-l", "random.log"]);
    assert_eq!(code, 0, "{}", err);
    let log = std::fs::read_to_string(d.join("random.log")).unwrap();
    let seed = log
        .split("[xezim] random seed: ")
        .nth(1)
        .and_then(|rest| rest.split(' ').next())
        .unwrap_or_else(|| panic!("no random seed line:\n{}", log));
    let values = |s: &str| -> Vec<String> {
        s.lines()
            .filter(|l| l.starts_with("r="))
            .map(str::to_string)
            .collect()
    };
    let replay = ok_stdout(&d, &["tb.sv", &format!("+seed={}", seed)]);
    assert_eq!(values(&replay), values(&log), "{}", log);
    assert_eq!(values(&log).len(), 4, "{}", log);
}

/// `-top <name>` is `-s <name>`, on the command line and in an args file.
#[test]
fn top_flag_is_s() {
    let d = filelist_top_fixture("top_flag");
    std::fs::write(d.join("files.f"), "roots.sv\n-top chosen_root\n").unwrap();
    for (args, tops) in [
        (&["roots.sv", "-top", "chosen_root"][..], &["chosen"][..]),
        (
            &["roots.sv", "-top", "chosen_root", "-s", "extra_root"],
            &["chosen", "extra"],
        ),
        (&["-f", "files.f"], &["chosen"]),
        (
            &["-f", "files.f", "-top", "extra_root"],
            &["chosen", "extra"],
        ),
    ] {
        assert_filelist_tops(&d, args, tops);
        let (_, _, err) = run_in(&d, args);
        assert!(!err.contains("unknown flag"), "{:?}: {}", args, err);
    }
    std::fs::write(d.join("bad.f"), "roots.sv\n-top\n").unwrap();
    let (code, out, err) = run_in(&d, &["-f", "bad.f"]);
    assert_ne!(code, 0, "{}{}", out, err);
    assert!(
        err.contains("-top requires a top module") && err.contains("bad.f"),
        "{}",
        err
    );
}

#[test]
fn log_file_holds_the_transcript() {
    let d = scratch("log");
    std::fs::write(d.join("tb.sv"), TIMED).unwrap();
    let native = ok_stdout(&d, &["tb.sv", "--max-time", "1ms"]);
    for flag in ["-l", "-logfile"] {
        let (code, out, err) = run_in(&d, &["tb.sv", flag, "run.log", "-do", "run -all"]);
        assert_eq!(code, 0);
        assert_eq!(out, "", "{} redirects: nothing on the terminal", flag);
        assert_eq!(err, "");
        assert_eq!(std::fs::read_to_string(d.join("run.log")).unwrap(), native);
    }
}

#[test]
fn full_reference_style_line_matches_native() {
    let d = scratch("full");
    std::fs::write(d.join("t.sv"), TOPS).unwrap();
    std::fs::write(d.join("r.sv"), RANDOM.replace("module tb;", "module rnd;")).unwrap();
    std::fs::write(d.join("files.f"), "t.sv\nr.sv\n").unwrap();
    let native = ok_stdout(
        &d,
        &[
            "-f",
            "files.f",
            "-s",
            "tb",
            "-s",
            "rnd",
            "+seed=5",
            "--max-time",
            "1000",
        ],
    );
    let compat = ok_stdout(
        &d,
        &[
            "-sv",
            "-F",
            "files.f",
            "-work",
            "work",
            "+acc",
            "-suppress",
            "2583",
            "-c",
            "-quiet",
            "-optargs=+acc",
            "-lib",
            "work",
            "tb",
            "rnd",
            "-sv_seed",
            "5",
            "-do",
            "run 1us; quit -f",
        ],
    );
    // `run 1us` ends the run at 1 us; the `--max-time` cap reports the last
    // event. Everything before that closing line is the same.
    let body = |s: &str| s[..s.rfind("Simulation finished").unwrap()].to_string();
    assert_eq!(body(&compat), body(&native));
    assert!(
        compat.ends_with("Simulation finished at time 1000\n"),
        "{}",
        compat
    );
}
