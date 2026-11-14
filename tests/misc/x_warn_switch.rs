//! `X_WARN` — opt-in warning the first time a signal takes an x bit after
//! time 0, naming the signal, the instance/module it lives in, and what drives
//! it.
//!
//! X after time 0 is the classic RTL-integration and gate-level failure: an
//! unconnected port, a register never reset, a bus with no active driver. A
//! waveform shows where x SURFACED, not what put it there, and tracing it back
//! by hand across a hierarchy is slow. This reports at the moment the bit turns
//! x, with the driver list, so the origin is one line of output instead of a
//! bisect.
//!
//! Off by default — x during initialization is normal and would bury the log.
//! Enabled by any of `X_WARN=1` (env), `+X_WARN` (plusarg), `--x-warn`,
//! `--X_WARN`, or `-X_WARN`; capped at 50 reports unless `X_WARN_LIMIT` /
//! `--x-warn-limit` / `+X_WARN_LIMIT=` says otherwise.
//!
//! Detection is hooked at every path that can write a signal — the `write_sig!`
//! macro (procedural / NBA / force), the comb-settle raw-bit fast copies, and
//! the bytecode VM's blocking-assign fast path. Each is guarded by one bool
//! load, so a run without the switch is unaffected. Missing any one of them
//! left whole driver classes silently unreported, which is why all three are
//! exercised below.

use std::process::Command;

fn xezim_bin() -> std::path::PathBuf {
    // Cargo names the binary it built for the integration tests.
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_xezim"))
}

/// Run the binary on `src` with extra args and env, returning stdout+stderr.
fn run(tag: &str, src: &str, args: &[&str], env: &[(&str, &str)]) -> String {
    let dir = std::env::temp_dir().join(format!("xezim_xwarn_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    let f = dir.join("t.sv");
    std::fs::write(&f, src).expect("write");
    let bin = xezim_bin();
    if !bin.exists() {
        let _ = std::fs::remove_dir_all(&dir);
        return String::new();
    }
    let mut cmd = Command::new(bin);
    cmd.arg("--simulate");
    for a in args {
        cmd.arg(a);
    }
    cmd.arg(&f);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run xezim");
    let _ = std::fs::remove_dir_all(&dir);
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// One x injected at t=10 propagates through a port, a register and an assign.
/// Every hop must be named, with the right module/instance and driver kind.
const CHAIN: &str = r#"
`timescale 1ns/1ns
module inner(input logic clk, input logic [3:0] a, output logic [3:0] y);
  logic [3:0] q;
  always_ff @(posedge clk) q <= a;
  assign y = q;
endmodule
module top;
  logic clk = 0;
  always #5 clk = ~clk;
  logic [3:0] src;
  logic [3:0] out;
  logic [3:0] combo;
  inner u_inner(.clk(clk), .a(src), .y(out));
  always_comb combo = src ^ 4'b0011;
  initial begin
    src = 4'b0101;
    #12 src = 4'bxx01;
    #20 $finish;
  end
endmodule
"#;

#[test]
fn off_by_default() {
    let out = run("off", CHAIN, &[], &[]);
    if out.is_empty() {
        return; // binary not built in this profile
    }
    assert!(
        !out.contains("warning] X on"),
        "must stay silent without the switch; got:\n{out}"
    );
}

#[test]
fn env_switch_reports_signal_module_and_drivers() {
    let out = run("env", CHAIN, &[], &[("XEZIM_X_WARN", "1")]);
    if out.is_empty() {
        return;
    }
    // The injected x and each hop it propagates to.
    assert!(out.contains("X on 'src'"), "the injected x; got:\n{out}");
    assert!(
        out.contains("X on 'u_inner.q'"),
        "the register that latched it; got:\n{out}"
    );
    assert!(
        out.contains("X on 'combo'"),
        "the always_comb that consumed it — this path bypasses write_sig! and \
         needs its own hook; got:\n{out}"
    );
    // Instance / module attribution.
    assert!(
        out.contains("in instance u_inner (module inner)"),
        "must name the instance and its module; got:\n{out}"
    );
    assert!(
        out.contains("in module top"),
        "top-level attribution; got:\n{out}"
    );
    // Driver attribution, one per kind.
    assert!(out.contains("always_ff"), "register driver; got:\n{out}");
    assert!(out.contains("always_comb"), "comb driver; got:\n{out}");
    assert!(
        out.contains("initial block") && out.contains("(procedural)"),
        "a procedurally-driven signal must be attributed to its process, not \
         called undriven; got:\n{out}"
    );
    // Which bits went x, and the rendered value.
    assert!(out.contains("bits [3:2]"), "the x bit range; got:\n{out}");
}

/// Every accepted spelling of the switch turns it on, and `X_WARN=0` does not.
#[test]
fn all_switch_spellings() {
    for (tag, args, env) in [
        ("flag", vec!["--x-warn"], vec![]),
        ("upper", vec!["--X_WARN"], vec![]),
        ("dash", vec!["-X_WARN"], vec![]),
        ("plus", vec!["+X_WARN"], vec![]),
        ("envv", vec![], vec![("XEZIM_X_WARN", "1")]),
        ("envon", vec![], vec![("XEZIM_X_WARN", "on")]),
    ] {
        let out = run(tag, CHAIN, &args, &env);
        if out.is_empty() {
            return;
        }
        assert!(
            out.contains("warning] X on"),
            "spelling {tag} must enable the warning; got:\n{out}"
        );
    }
    let off = run("envoff", CHAIN, &[], &[("XEZIM_X_WARN", "0")]);
    if !off.is_empty() {
        assert!(
            !off.contains("warning] X on"),
            "X_WARN=0 must stay off; got:\n{off}"
        );
    }
}

/// The report cap is honoured and announces itself, via both spellings.
#[test]
fn report_limit_is_capped_and_announced() {
    let out = run(
        "lim",
        CHAIN,
        &[],
        &[("XEZIM_X_WARN", "1"), ("XEZIM_X_WARN_LIMIT", "1")],
    );
    if out.is_empty() {
        return;
    }
    assert_eq!(
        out.matches("warning] X on").count(),
        1,
        "limit 1 means exactly one report; got:\n{out}"
    );
    assert!(
        out.contains("limit of 1 reached"),
        "the cap must say it suppressed the rest; got:\n{out}"
    );

    let plus = run("limplus", CHAIN, &["+X_WARN_LIMIT=1"], &[]);
    if !plus.is_empty() {
        assert_eq!(
            plus.matches("warning] X on").count(),
            1,
            "+X_WARN_LIMIT= also enables and caps; got:\n{plus}"
        );
    }
}

/// Each signal is named once even though a clocked register re-loads the same
/// x on every edge — otherwise a long run drowns in duplicates.
#[test]
fn each_signal_reported_once() {
    let src = r#"
`timescale 1ns/1ns
module top;
  logic clk = 0;
  always #5 clk = ~clk;
  logic [3:0] src, q;
  always_ff @(posedge clk) q <= src;
  initial begin
    src = 4'b0000;
    #12 src = 4'bxxxx;   // stays x for many edges
    #200 $finish;
  end
endmodule
"#;
    let out = run("once", src, &[], &[("XEZIM_X_WARN", "1")]);
    if out.is_empty() {
        return;
    }
    assert_eq!(
        out.matches("X on 'q'").count(),
        1,
        "one report per signal, not per edge; got:\n{out}"
    );
}

/// Two deliberate exclusions, both documented behaviour rather than gaps:
///
///  * z is not reported. An undriven NET reads z (§6.6.1) and z is a legitimate
///    steady state for anything tri-state, so warning on it would bury the
///    real signal.
///  * a signal that has been x since initialization is not reported. The switch
///    reports a NEW x — a signal that held a defined value and then went x,
///    which is the failure worth chasing. X during reset/init is normal.
#[test]
fn z_and_init_x_are_excluded_but_a_new_x_is_reported() {
    let src = r#"
`timescale 1ns/1ns
module top;
  logic clk = 0;
  always #5 clk = ~clk;
  wire  [3:0] undriven_net;     // reads z
  logic [3:0] undriven_var;     // never assigned: x from t=0
  logic [3:0] tap_z, tap_x, tap_new;
  logic       go;
  always_ff @(posedge clk) if (go) tap_z <= undriven_net;
  always_ff @(posedge clk) if (go) tap_x <= undriven_var;
  // Defined first, then takes x — the case the switch exists for.
  always_ff @(posedge clk) tap_new <= go ? undriven_var : 4'b0000;
  initial begin
    go = 0;
    #40 go = 1;
    #40 $finish;
  end
endmodule
"#;
    let out = run("excl", src, &[], &[("XEZIM_X_WARN", "1")]);
    if out.is_empty() {
        return;
    }
    assert!(
        !out.contains("X on 'tap_z'"),
        "z must not be reported as x; got:\n{out}"
    );
    assert!(
        !out.contains("X on 'tap_x'"),
        "a signal x since init has no NEW-x event; got:\n{out}"
    );
    assert!(
        out.contains("X on 'tap_new'"),
        "defined-then-x is exactly what must be reported; got:\n{out}"
    );
    assert!(
        out.contains("always_ff"),
        "attributed to the register that took the x; got:\n{out}"
    );
}

/// A signal that has been x (or z) since the beginning must never be blamed,
/// even when the x pattern changes or first materializes at t>0 through an
/// undriven input. Only a bit that held a VALID 0/1 and then went x reports.
#[test]
fn x_from_beginning_is_silent_valid_to_x_reports() {
    let src = r#"
`timescale 1ns/1ns
module top(input logic din);         // undriven input: x/z from the start
  logic clk = 0;
  always #5 clk = ~clk;
  logic q1;                          // captures x forever: silent
  logic mid;
  assign mid = din & 1'b1;           // x through comb logic: silent
  logic q2 = 0;                      // valid then corrupted: REPORT
  logic q3;                          // x, then valid, then x: REPORT
  always @(posedge clk) q1 <= din;
  initial begin
    #7  q3 = 1'b1;
    #10 q2 = 1'bx;
    #10 q3 = 1'bx;
    #10 $finish;
  end
endmodule
"#;
    let out = run("frombeg", src, &[], &[("XEZIM_X_WARN", "1")]);
    if out.is_empty() {
        return;
    }
    assert!(
        !out.contains("X on 'q1'") && !out.contains("X on 'mid'") && !out.contains("X on 'din'"),
        "x-from-beginning signals must stay silent; got:\n{out}"
    );
    assert!(
        out.contains("X on 'q2'"),
        "valid 0 corrupted to x must report; got:\n{out}"
    );
    assert!(
        out.contains("X on 'q3'"),
        "x -> valid -> x must report the corruption; got:\n{out}"
    );
}
