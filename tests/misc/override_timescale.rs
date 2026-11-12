//! `-override_timescale <unit>/<prec>`: one timescale for every design
//! element, package and compilation unit. Every `timescale directive,
//! `timeunit`/`timeprecision` declaration and `--module-timescale` is
//! replaced; time literals with a unit (`#3ns`) keep their absolute value.

use std::path::{Path, PathBuf};
use std::process::Command;

fn xezim() -> String {
    // Cargo names the binary it built for the integration tests.
    env!("CARGO_BIN_EXE_xezim").to_string()
}

/// A fresh scratch directory per test (tests run in parallel).
fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("xezim_override_ts_{}_{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Run xezim in `dir`; returns (exit code, the `T|` lines, stderr).
fn run(dir: &Path, args: &[&str]) -> (i32, Vec<String>, String) {
    let out = Command::new(xezim())
        .current_dir(dir)
        .args(args)
        .output()
        .expect("run xezim");
    let stdout = String::from_utf8_lossy(&out.stdout);
    (
        out.status.code().unwrap_or(-1),
        stdout
            .lines()
            .filter(|l| l.starts_with("T|"))
            .map(str::to_string)
            .collect(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// Four timescales in one design: a `timescale 1ps/1ps` module, a module
/// declaring `timeunit 1us; timeprecision 1ns;`, a package with its own
/// `timeunit`, and a top with none of its own (it inherits the directive).
const DESIGN: &str = r#"
package pk;
  timeunit 10ns; timeprecision 1ns;
  task automatic wait1(); #1; endtask
endpackage

`timescale 1ps/1ps
module ps_mod;
  initial begin
    #1000 $display("T|ps_mod t=%0t time=%0d", $realtime, $time);
  end
endmodule

module us_mod;
  timeunit 1us; timeprecision 1ns;
  initial begin
    #2 $display("T|us_mod time=%0d", $time);
  end
endmodule

module top;
  ps_mod a();
  us_mod b();
  initial begin
    pk::wait1();
    $display("T|pkg_wait time=%0d", $time);
    #3ns $display("T|abs3ns time=%0d", $time);
    #5us $finish;
  end
endmodule
"#;

fn design_dir(tag: &str) -> PathBuf {
    let d = scratch(tag);
    std::fs::write(d.join("tb.sv"), DESIGN).unwrap();
    d
}

#[test]
fn without_the_switch_each_element_keeps_its_own_timescale() {
    let d = design_dir("none");
    let (rc, lines, err) = run(&d, &["-s", "top", "tb.sv"]);
    assert_eq!(rc, 0, "stderr:\n{}", err);
    // ps_mod: #1000 ps = 1 ns; pk: #1 x 10 ns; top (1ps after the
    // directive): $time in ps; us_mod: #2 us.
    assert_eq!(
        lines,
        vec![
            "T|ps_mod t=1000 time=1000",
            "T|pkg_wait time=10000",
            "T|abs3ns time=13000",
            "T|us_mod time=2",
        ],
        "stderr:\n{}",
        err
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn override_1ns_replaces_every_timescale() {
    let d = design_dir("ns");
    let (rc, lines, err) = run(
        &d,
        &["-override_timescale", "1ns/1ns", "-s", "top", "tb.sv"],
    );
    assert_eq!(rc, 0, "stderr:\n{}", err);
    // Every element counts in ns now: the package's #1 is 1 ns, us_mod's #2
    // is 2 ns, ps_mod's #1000 is 1000 ns; #3ns stays 3 ns.
    assert_eq!(
        lines,
        vec![
            "T|pkg_wait time=1",
            "T|us_mod time=2",
            "T|abs3ns time=4",
            "T|ps_mod t=1000 time=1000",
        ],
        "stderr:\n{}",
        err
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn override_1ps_counts_bare_delays_in_ps_and_keeps_absolute_literals() {
    let d = design_dir("ps");
    let (rc, lines, err) = run(
        &d,
        &["-override_timescale", "1ps/1ps", "-s", "top", "tb.sv"],
    );
    assert_eq!(rc, 0, "stderr:\n{}", err);
    // pk #1 = 1 ps, us_mod #2 = 2 ps, #3ns = 3000 ps (absolute), ps_mod
    // #1000 = 1000 ps.
    assert_eq!(
        lines,
        vec![
            "T|pkg_wait time=1",
            "T|us_mod time=2",
            "T|ps_mod t=1000 time=1000",
            "T|abs3ns time=3001",
        ],
        "stderr:\n{}",
        err
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn every_spelling_and_args_files_are_accepted() {
    let d = design_dir("spell");
    let want = run(
        &d,
        &["-override_timescale", "1ns/1ns", "-s", "top", "tb.sv"],
    )
    .1;
    for args in [
        vec!["--override_timescale", "1ns/1ns"],
        vec!["-override-timescale", "1ns/1ns"],
        vec!["--override-timescale=1ns/1ns"],
        vec!["-override_timescale=1ns/1ns"],
    ] {
        let mut all = args.clone();
        all.extend(["-s", "top", "tb.sv"]);
        let (rc, lines, err) = run(&d, &all);
        assert_eq!(rc, 0, "{:?}: {}", args, err);
        assert_eq!(lines, want, "{:?}", args);
    }
    // Inside an args file, and winning over --module-timescale / -timescale.
    std::fs::write(
        d.join("run.f"),
        "-timescale 1ps/1ps\n-override_timescale 1ns/1ns\n-s top\ntb.sv\n",
    )
    .unwrap();
    let (rc, lines, err) = run(&d, &["-f", "run.f"]);
    assert_eq!(rc, 0, "{}", err);
    assert_eq!(lines, want);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_bad_value_is_an_error() {
    let d = design_dir("bad");
    let (rc, _, err) = run(
        &d,
        &["-override_timescale", "1ps/1ns", "-s", "top", "tb.sv"],
    );
    assert_eq!(rc, 1, "stderr:\n{}", err);
    assert!(err.contains("-override_timescale"), "stderr:\n{}", err);
    let (rc, _, err) = run(&d, &["-override_timescale", "fast", "-s", "top", "tb.sv"]);
    assert_eq!(rc, 1, "stderr:\n{}", err);
    let _ = std::fs::remove_dir_all(&d);
}
