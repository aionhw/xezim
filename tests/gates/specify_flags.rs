//! Commercial gate-level-sim flags: `+nospecify` suppresses specify-block
//! module path delays (zero-delay GLS) and timing checks;
//! `+notimingcheck`/`+notimingchecks` suppress only the §31 timing checks;
//! `+no_notifier` / `+no_tchk_msg` keep one of a violation's two effects.
//! The `-` spellings are accepted too. CLI-level tests because the
//! switches are process-globals set by argument parsing.

use std::path::PathBuf;
use std::process::Command;

fn xezim_bin() -> PathBuf {
    // Cargo names the binary it built for the integration tests.
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_xezim"))
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

/// Default: the (a => y) = 10 path delay defers y — two ns after the input
/// edge it still holds x, because the time-0 x->0 change also waits for the
/// path delay and the 1 at 5 ns replaced it — and the 1 arrives by the end.
/// Cross-checked against the reference simulator.
#[test]
fn specify_path_delay_applies_by_default() {
    let out = run("default", &[]);
    assert!(out.contains("MID y=x"), "path delay must defer y:\n{}", out);
    assert!(
        out.contains("END y=1"),
        "y must eventually arrive:\n{}",
        out
    );
}

/// `+nospecify` (and `-nospecify`): zero-delay — y flips immediately.
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

/// The timing-check disables are recognized — no unknown-flag warning — and
/// leave path delays alone.
#[test]
fn notimingcheck_keeps_path_delays() {
    for flag in ["+notimingcheck", "+notimingchecks", "-notimingchecks"] {
        let out = run("ntc", &[flag]);
        assert!(
            !out.to_lowercase().contains("unknown flag"),
            "{} must be recognized:\n{}",
            flag,
            out
        );
        assert!(
            out.contains("MID y=x"),
            "{} must not change timing:\n{}",
            flag,
            out
        );
    }
}

const TCHK_SRC: &str = "`timescale 1ns/1ns
module ff(input d, input clk);
  reg n = 0;
  always @(n) $display(\"N=%b\", n);
  specify $setup(d, posedge clk, 3, n); endspecify
endmodule
module tb;
  reg d = 0, clk = 0;
  ff u(.d(d), .clk(clk));
  initial begin #10 d = 1; #1 clk = 1; #5 $finish; end
endmodule
";

/// §31 timing checks: a setup violation prints one error and toggles the
/// notifier (the reference simulator reports the same violation). The
/// disables drop both effects or one of them, and `--error-exit` counts the
/// violation as an error.
#[test]
fn timing_check_flags() {
    let dir = std::env::temp_dir().join(format!("xezim_tchk_flags_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let sv = dir.join("tc.sv");
    std::fs::write(&sv, TCHK_SRC).unwrap();
    let run = |args: &[&str]| -> (Option<i32>, String) {
        let o = Command::new(xezim_bin())
            .args(args)
            .arg(&sv)
            .output()
            .unwrap();
        (
            o.status.code(),
            format!(
                "{}{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            ),
        )
    };
    let msg = "$setup( d:10 ns, posedge clk:11 ns, 3 ns ) violation in tb.u at time 11 ns";
    let (code, out) = run(&[]);
    assert!(
        out.contains(msg) && out.contains("N=1"),
        "default:\n{}",
        out
    );
    assert_eq!(
        code,
        Some(0),
        "a violation alone does not fail the run:\n{}",
        out
    );
    let (code, out) = run(&["--error-exit"]);
    assert_eq!(code, Some(1), "--error-exit counts the violation:\n{}", out);
    for flag in [
        "+notimingcheck",
        "+notimingchecks",
        "-notimingchecks",
        "+nospecify",
    ] {
        let (_, out) = run(&[flag]);
        assert!(
            !out.contains("violation") && !out.contains("N=1"),
            "{} must disable the check:\n{}",
            flag,
            out
        );
    }
    let (_, out) = run(&["+no_notifier"]);
    assert!(
        out.contains(msg) && !out.contains("N=1"),
        "+no_notifier:\n{}",
        out
    );
    let (_, out) = run(&["+no_tchk_msg"]);
    assert!(
        !out.contains("violation") && out.contains("N=1"),
        "+no_tchk_msg:\n{}",
        out
    );
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
/// path delay to zero. y reads x until its first path delay elapses (the
/// reference simulator's output for all three selections).
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
    assert!(
        typ.contains("T4 y=x") && typ.contains("T8 y=1"),
        "typ default (5ns):\n{}",
        typ
    );
    let min = run(&["+mindelays"]);
    assert!(min.contains("T4 y=1"), "+mindelays (2ns):\n{}", min);
    let max = run(&["+maxdelays"]);
    assert!(
        max.contains("T8 y=x") && max.contains("T12 y=1"),
        "+maxdelays (9ns):\n{}",
        max
    );
}

/// The should-fail lint must see inside generate constructs — every rule was
/// bypassed by wrapping illegal code in `generate begin ... end`.
#[test]
fn lint_checks_apply_inside_generate() {
    let bad =
        "module t; reg fb = 0;\n generate begin : g always fb = ~fb; end endgenerate\nendmodule\n";
    assert!(
        xezim::simulate(bad, 1000).is_err(),
        "illegal always inside generate must be rejected"
    );
    let good =
        "module t; reg c = 0;\n generate begin : g always #10 c = ~c; end endgenerate\nendmodule\n";
    assert!(
        xezim::simulate(good, 100).is_ok(),
        "legal always inside generate must stay accepted"
    );
}

/// GLS structural-delay modes (commercial-simulator convention). `+delay_mode_zero` forces
/// all structural (specify/SDF) delays to 0; `+delay_mode_unit` collapses each
/// nonzero one to 1 tick. Default keeps the specify path delay. These used to
/// fall silently into the generic plusarg bucket.
#[test]
fn delay_mode_zero_and_unit() {
    fn xbin() -> PathBuf {
        // Cargo names the binary it built for the integration tests.
        std::path::PathBuf::from(env!("CARGO_BIN_EXE_xezim"))
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
        format!(
            "{}{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        )
    };
    // default: specify 8ns → edge at t9, so y is still x at t3 (the time-0
    // value waits for the path delay too; the reference simulator agrees).
    assert!(run(&[]).contains("T3 y=x"), "default specify delay");
    // zero: edge at t1 → y=1 at t3.
    assert!(
        run(&["+delay_mode_zero"]).contains("T3 y=1"),
        "+delay_mode_zero"
    );
    // unit: 8→1, edge at t2 → y=1 at t3.
    assert!(
        run(&["+delay_mode_unit"]).contains("T3 y=1"),
        "+delay_mode_unit"
    );
}
