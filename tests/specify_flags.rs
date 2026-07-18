//! Commercial gate-level-sim flags: `+nospecify` suppresses specify-block
//! module path delays (zero-delay GLS); `+notimingcheck`/`+notimingchecks`
//! are accepted as documented no-ops (xezim does not model specify timing
//! checks, so they are permanently "disabled" already). Xcelium's `-`
//! spellings are accepted for both. CLI-level tests because the switch is a
//! process-global set by argument parsing.

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

const SRC: &str = "`timescale 1ns/1ns
module buf1(input a, output y);
  assign y = a;
  specify (a => y) = 10; endspecify
endmodule
module tb;
  reg a = 0; wire y;
  buf1 u(.a(a), .y(y));
  initial begin
    #5 a = 1;
    #2 $display(\"MID y=%b\", y);
    #10 $display(\"END y=%b\", y);
    $finish;
  end
endmodule
";

/// Each caller passes a unique tag: these tests run in PARALLEL inside one
/// binary, and sharing a single sp.sv let `fs::write`'s truncate-then-write
/// race a concurrently spawned xezim reading a partial file (intermittent
/// missing-output failures).
fn run(tag: &str, args: &[&str]) -> String {
    let dir = std::env::temp_dir().join(format!("xezim_specify_flags_{}", tag));
    std::fs::create_dir_all(&dir).unwrap();
    let sv = dir.join("sp.sv");
    std::fs::write(&sv, SRC).unwrap();
    let out = Command::new(xezim_bin())
        .args(args)
        .arg(&sv)
        .output()
        .expect("run xezim");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// Default: the (a => y) = 10 path delay holds y at 0 two ns after the input
/// edge, and it arrives by the end.
#[test]
fn specify_path_delay_applies_by_default() {
    let out = run("default", &[]);
    assert!(out.contains("MID y=0"), "path delay must defer y:\n{}", out);
    assert!(out.contains("END y=1"), "y must eventually arrive:\n{}", out);
}

/// `+nospecify` (and Xcelium's `-nospecify`): zero-delay — y flips immediately.
#[test]
fn nospecify_suppresses_path_delays() {
    for flag in ["+nospecify", "-nospecify"] {
        let out = run("nospec", &[flag]);
        assert!(
            out.contains("MID y=1"),
            "{} must suppress the specify path delay:\n{}",
            flag,
            out
        );
    }
}

/// The timing-check disables are recognized no-ops — no unknown-flag warning,
/// simulation unchanged.
#[test]
fn notimingcheck_is_a_quiet_noop() {
    for flag in ["+notimingcheck", "+notimingchecks", "-notimingchecks"] {
        let out = run("ntc", &[flag]);
        assert!(
            !out.to_lowercase().contains("unknown flag"),
            "{} must be recognized:\n{}",
            flag,
            out
        );
        assert!(out.contains("MID y=0"), "{} must not change timing:\n{}", flag, out);
    }
}

const TRIPLET_SRC: &str = "`timescale 1ns/1ns
module buf2(input a, output y);
  assign y = a;
  specify (a => y) = (2:5:9); endspecify
endmodule
module tb;
  reg a = 0; wire y;
  buf2 u(.a(a), .y(y));
  initial begin
    #1 a = 1;
    #3 $display(\"T4 y=%b\", y);
    #4 $display(\"T8 y=%b\", y);
    #4 $display(\"T12 y=%b\", y);
    $finish;
  end
endmodule
";

/// min:typ:max triplets: default typ, +mindelays/+maxdelays select the ends.
/// A triplet used to derail the specify parser and silently drop the whole
/// path delay to zero.
#[test]
fn min_typ_max_triplet_selection() {
    let dir = std::env::temp_dir().join("xezim_specify_triplet");
    std::fs::create_dir_all(&dir).unwrap();
    let sv = dir.join("mtm.sv");
    std::fs::write(&sv, TRIPLET_SRC).unwrap();
    let run = |args: &[&str]| -> String {
        let out = std::process::Command::new(xezim_bin())
            .args(args)
            .arg(&sv)
            .output()
            .expect("run");
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    };
    let typ = run(&[]);
    assert!(typ.contains("T4 y=0") && typ.contains("T8 y=1"), "typ default (5ns):\n{}", typ);
    let min = run(&["+mindelays"]);
    assert!(min.contains("T4 y=1"), "+mindelays (2ns):\n{}", min);
    let max = run(&["+maxdelays"]);
    assert!(max.contains("T8 y=0") && max.contains("T12 y=1"), "+maxdelays (9ns):\n{}", max);
}

/// The should-fail lint must see inside generate constructs — every rule was
/// bypassed by wrapping illegal code in `generate begin ... end`.
#[test]
fn lint_checks_apply_inside_generate() {
    let bad = "module t; reg fb = 0;\n generate begin : g always fb = ~fb; end endgenerate\nendmodule\n";
    assert!(
        xezim::simulate(bad, 1000).is_err(),
        "illegal always inside generate must be rejected"
    );
    let good = "module t; reg c = 0;\n generate begin : g always #10 c = ~c; end endgenerate\nendmodule\n";
    assert!(
        xezim::simulate(good, 100).is_ok(),
        "legal always inside generate must stay accepted"
    );
}

/// GLS structural-delay modes (VCS/Questa/Xcelium). `+delay_mode_zero` forces
/// all structural (specify/SDF) delays to 0; `+delay_mode_unit` collapses each
/// nonzero one to 1 tick. Default keeps the specify path delay. These used to
/// fall silently into the generic plusarg bucket.
#[test]
fn delay_mode_zero_and_unit() {
    fn xbin() -> PathBuf {
        let mut p = std::env::current_exe().unwrap();
        p.pop();
        if p.ends_with("deps") { p.pop(); }
        p.join("xezim")
    }
    let dir = std::env::temp_dir().join(format!("xezim_delaymode_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let sv = dir.join("sp.sv");
    std::fs::write(
        &sv,
        "`timescale 1ns/1ns\n\
         module cbuf(input a, output y); assign y=a; specify (a=>y)=8; endspecify endmodule\n\
         module t; reg a=0; wire y; cbuf u(.a(a),.y(y));\n\
         initial begin #1 a=1; #2 $display(\"T3 y=%b\",y); #8 $display(\"T11 y=%b\",y); $finish; end\n\
         endmodule\n",
    )
    .unwrap();
    let run = |args: &[&str]| -> String {
        let o = Command::new(xbin()).args(args).arg(&sv).output().unwrap();
        format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
    };
    // default: specify 8ns → edge at t9, so y=0 at t3.
    assert!(run(&[]).contains("T3 y=0"), "default specify delay");
    // zero: edge at t1 → y=1 at t3.
    assert!(run(&["+delay_mode_zero"]).contains("T3 y=1"), "+delay_mode_zero");
    // unit: 8→1, edge at t2 → y=1 at t3.
    assert!(run(&["+delay_mode_unit"]).contains("T3 y=1"), "+delay_mode_unit");
}
