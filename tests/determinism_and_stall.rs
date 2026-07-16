//! Two properties a simulator has to have to be debuggable at all:
//! a run you can repeat, and a run that ends.
//!
//! §18.14 (random stability) is written around the idea that a seed reproduces
//! a stream. That is only useful if the DEFAULT run is seeded too — a simulator
//! that pulls a fresh seed from the OS on every launch cannot replay the random
//! failure it just reported. So the default seed is a fixed constant, `+seed=<n>`
//! selects another stream, and `+seed=random` opts into entropy (printing the
//! seed it drew, so that run can be replayed with `+seed=<that>`).
//!
//! §4.4.2.3 lets a process re-arm at the current timestamp (`#0`, an event
//! ping-pong, a `wait` on an already-true condition). Nothing in the LRM bounds
//! how often — so a design can livelock the scheduler at one timestamp and time
//! never advances. xezim used to spin there forever, printing nothing; the user
//! just sees "stuck at time 0". It now stops and names the offending processes.

use xezim::simulate_multi;

fn run(src: &str, plusargs: &[String]) -> xezim::compiler::Simulator {
    simulate_multi(
        &[src.to_string()],
        100_000,
        None,
        &[],
        &[],
        None,
        false,
        None,
        None,
        &[],
        plusargs,
        1,
        None,
        &[],
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
    )
    .expect("simulate failed")
}

/// A design that randomizes with no seed given must produce the SAME values on
/// every run. (Before this, the RNG was seeded from entropy and two runs of the
/// same design disagreed — so a random failure could not be reproduced.)
#[test]
fn default_seed_is_deterministic_across_runs() {
    const SRC: &str = r#"
module tb;
  int vals[8];
  initial begin
    foreach (vals[i]) vals[i] = $urandom();
    $display("SEQ %0d %0d %0d %0d %0d %0d %0d %0d",
             vals[0], vals[1], vals[2], vals[3],
             vals[4], vals[5], vals[6], vals[7]);
  end
endmodule
"#;
    let seq = |sim: &xezim::compiler::Simulator| -> String {
        sim.output
            .iter()
            .find(|o| o.message.starts_with("SEQ "))
            .expect("no SEQ line")
            .message
            .clone()
    };

    let a = seq(&run(SRC, &[]));
    let b = seq(&run(SRC, &[]));
    assert_eq!(a, b, "an unseeded run must be reproducible, not entropy-seeded");

    // An explicit seed reproduces its own stream ...
    let s7a = seq(&run(SRC, &["seed=7".to_string()]));
    let s7b = seq(&run(SRC, &["seed=7".to_string()]));
    assert_eq!(s7a, s7b, "+seed=7 must reproduce the same stream");

    // ... and a different seed is actually a different stream.
    let s8 = seq(&run(SRC, &["seed=8".to_string()]));
    assert_ne!(s7a, s8, "+seed=7 and +seed=8 must differ");
}

/// A `randomize()` sequence — not just `$urandom` — must also be reproducible
/// by default, since that is what a constrained-random regression replays.
#[test]
fn default_seed_reproduces_randomize_sequence() {
    const SRC: &str = r#"
class P;
  rand bit [7:0] a;
  constraint c { a inside {[1:200]}; }
endclass
module tb;
  initial begin
    P p = new();
    string s = "";
    repeat (10) begin
      void'(p.randomize());
      s = {s, $sformatf("%0d,", p.a)};
    end
    $display("R %s", s);
  end
endmodule
"#;
    let r = |sim: &xezim::compiler::Simulator| -> String {
        sim.output
            .iter()
            .find(|o| o.message.starts_with("R "))
            .expect("no R line")
            .message
            .clone()
    };
    assert_eq!(
        r(&run(SRC, &[])),
        r(&run(SRC, &[])),
        "randomize() must replay identically on an unseeded run"
    );
}

/// A zero-delay loop (`forever #0`) re-arms at the same timestamp forever. The
/// run must TERMINATE with a diagnostic rather than spin until the user kills
/// it — `simulate` returning at all is the assertion here.
#[test]
fn zero_delay_livelock_terminates_instead_of_hanging() {
    const SRC: &str = r#"
module tb;
  int n = 0;
  initial forever begin
    #0;
    n++;
  end
endmodule
"#;
    std::env::set_var("XEZIM_STALL_LIMIT", "2000");
    let sim = run(SRC, &[]);
    std::env::remove_var("XEZIM_STALL_LIMIT");

    // It stalled at time 0 — it must not have silently "finished" at some later
    // time, and it must not have hung (reaching this line proves that).
    assert_eq!(sim.time, 0, "a #0 livelock cannot advance time");
    let n = sim
        .get_signal("tb.n")
        .or_else(|| sim.get_signal("n"))
        .and_then(|v| v.to_u64())
        .expect("n not readable");
    // The final delta cycle is cut off before the body completes, so the count
    // lands one short of the limit.
    assert!(
        n >= 1990,
        "the loop should have spun up to the stall limit before being cut off, got {}",
        n
    );
}

/// The same livelock reached through `wait` on an already-true condition — the
/// shape that used to blow the stack instead of hanging.
#[test]
fn wait_on_true_condition_in_forever_terminates() {
    const SRC: &str = r#"
module tb;
  bit ready = 1;
  int n = 0;
  initial forever begin
    wait (ready);
    n++;
  end
endmodule
"#;
    std::env::set_var("XEZIM_STALL_LIMIT", "2000");
    let sim = run(SRC, &[]);
    std::env::remove_var("XEZIM_STALL_LIMIT");
    assert_eq!(sim.time, 0, "wait-on-true livelock cannot advance time");
}

/// The stall report must name the RTL behind each spinner, not just a bare
/// pid: the creating construct's kind + file:line, the instance path, and the
/// re-arm reason. Asserted through the CLI binary (the report goes to stderr,
/// which the in-process `simulate_multi` harness can't capture) — same
/// subprocess pattern as tests/log_redirect.rs.
#[test]
fn stall_report_names_the_rtl_behind_each_spinner() {
    use std::process::Command;

    fn xezim_bin() -> std::path::PathBuf {
        // target/release/deps/<test binary> -> target/release/xezim
        let mut p = std::env::current_exe().expect("current_exe");
        p.pop();
        if p.ends_with("deps") {
            p.pop();
        }
        p.join("xezim")
    }

    let dir = std::env::temp_dir().join("xezim_stall_report_test");
    std::fs::create_dir_all(&dir).expect("mkdir");

    // Shape (a): `initial forever begin #0; ... end` in the top module.
    // The `initial` sits on line 3 of the file.
    let sv_a = dir.join("stall_shape_a.sv");
    std::fs::write(
        &sv_a,
        "module tb;\n  int n = 0;\n  initial forever begin\n    #0;\n    n++;\n  end\nendmodule\n",
    )
    .expect("write sv");
    let out = Command::new(xezim_bin())
        .env("XEZIM_STALL_LIMIT", "1000")
        .arg(&sv_a)
        .output()
        .expect("run xezim");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("simulation STALLED"), "no stall report:\n{}", stderr);
    assert!(
        stderr.contains(&format!("initial block at {}:3", sv_a.display())),
        "offender line must carry the construct kind and file:line:\n{}",
        stderr
    );
    assert!(
        stderr.contains("re-arming via #0 delay"),
        "offender line must classify the #0 re-arm:\n{}",
        stderr
    );
    assert!(
        stderr.contains("ran 1000 times at this timestamp"),
        "the established count phrasing must survive:\n{}",
        stderr
    );

    // The same livelock buried two instances deep: the offender must be
    // located by INSTANCE PATH, not just by module/file.
    let sv_d = dir.join("stall_shape_d.sv");
    std::fs::write(
        &sv_d,
        "module leaf;\n  int n = 0;\n  initial forever begin\n    #0;\n    n++;\n  end\nendmodule\nmodule mid;\n  leaf u_leaf();\nendmodule\nmodule top;\n  mid u_mid();\nendmodule\n",
    )
    .expect("write sv");
    let out = Command::new(xezim_bin())
        .env("XEZIM_STALL_LIMIT", "1000")
        .arg(&sv_d)
        .output()
        .expect("run xezim");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("(top.u_mid.u_leaf, module leaf)"),
        "offender must be named by its instance path AND defining module:\n{}",
        stderr
    );
    assert!(
        stderr.contains(&format!("initial block at {}:3", sv_d.display())),
        "nested offender must still resolve to file:line:\n{}",
        stderr
    );
    assert!(stderr.contains("re-arming via #0 delay"), "{}", stderr);
}

/// A MULTI-FILE design must still get file:line in the stall report. Spans
/// are per-file byte offsets, so with several files an offset alone cannot
/// name its file — the report used to silently drop the location whenever
/// more than one file was long enough to contain the offset. The offender's
/// instance scope names its defining module, and the elaborator now records
/// which file defined each module, so the span resolves against exactly that
/// file's text.
#[test]
fn stall_report_resolves_file_line_in_multi_file_designs() {
    use std::process::Command;

    fn xezim_bin() -> std::path::PathBuf {
        let mut p = std::env::current_exe().expect("current_exe");
        p.pop();
        if p.ends_with("deps") {
            p.pop();
        }
        p.join("xezim")
    }

    let dir = std::env::temp_dir().join("xezim_stall_report_multifile_test");
    std::fs::create_dir_all(&dir).expect("mkdir");

    // f1: an uninitialized-real clock period — the classic accidental
    // zero-delay livelock. The `always` sits on line 4 of f1.
    let f1 = dir.join("stall_mf_dut.sv");
    std::fs::write(
        &f1,
        "module dut;\n  real p;\n  reg clk = 0;\n  always #(p/2) clk = ~clk;\nendmodule\n",
    )
    .expect("write f1");
    // f2: the top, PADDED so it is longer than f1's span offsets — the
    // exact-fit fallback alone would then refuse to resolve (ambiguous).
    let f2 = dir.join("stall_mf_top.sv");
    std::fs::write(
        &f2,
        "module top;\n  localparam int PAD0 = 0;\n  localparam int PAD1 = 1;\n  localparam int PAD2 = 2;\n  localparam int PAD3 = 3;\n  dut u_dut();\nendmodule\n",
    )
    .expect("write f2");

    let out = Command::new(xezim_bin())
        .env("XEZIM_STALL_LIMIT", "1000")
        .arg(&f1)
        .arg(&f2)
        .output()
        .expect("run xezim");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("simulation STALLED"), "no stall report:\n{}", stderr);
    assert!(
        stderr.contains(&format!("always block at {}:4", f1.display())),
        "multi-file offender must resolve to its OWN file's line:\n{}",
        stderr
    );
    assert!(
        stderr.contains("(top.u_dut, module dut)"),
        "instance path must carry the defining module's name:\n{}",
        stderr
    );
    assert!(
        stderr.contains("re-arming via"),
        "offender line must classify the re-arm:\n{}",
        stderr
    );
    // The zero-valued delay EXPRESSION must be quoted — `#(p/2)` with an
    // uninitialized real period is the classic accidental livelock, and
    // "re-arming via #0 delay" alone hides where the zero comes from.
    assert!(
        stderr.contains("re-arming via #(p/2) — currently 0"),
        "the delay expression and its zero value must be shown:\n{}",
        stderr
    );
    assert!(
        stderr.contains("ran 1000 times at this timestamp"),
        "the established count phrasing must survive:\n{}",
        stderr
    );
}

/// The detector must not fire on a design that merely uses several delta cycles
/// at one timestamp — NBA settling, `#0` used once, zero-delay fork/join are all
/// legal and common.
#[test]
fn legitimate_delta_cycles_do_not_trip_the_detector() {
    const SRC: &str = r#"
module tb;
  logic a, b, c;
  int done = 0;
  always @(a) b = ~a;
  always @(b) c = ~b;
  initial begin
    a = 0;
    #0 a = 1;      // a legal single #0
    fork
      #0 done++;   // zero-delay fork
      #0 done++;
    join
    #1 done++;
    #10 $finish;
  end
endmodule
"#;
    let sim = run(SRC, &[]);
    assert!(sim.time >= 1, "a normal design must advance time, got {}", sim.time);
    let done = sim
        .get_signal("tb.done")
        .or_else(|| sim.get_signal("done"))
        .and_then(|v| v.to_u64())
        .expect("done not readable");
    assert_eq!(done, 3, "all zero-delay work must still run");
}

/// The report's "Parked" section must name the process the spinner is waiting
/// for — the classic shape: a clock generator spinning on a zero period whose
/// setter is scheduled at a future time the spin never reaches.
#[test]
fn stall_report_lists_parked_setter() {
    let dir = std::env::temp_dir().join("xezim_stall_parked");
    std::fs::create_dir_all(&dir).expect("mkdir");
    let sv = dir.join("parked.sv");
    std::fs::write(
        &sv,
        "`timescale 1ps/1ps\nmodule t; real p; reg clk = 0;\n\
         always #(p/2) clk = ~clk;\n\
         initial begin #1 p = 100.0; end\nendmodule\n",
    )
    .expect("write sv");

    let mut bin = std::env::current_exe().unwrap();
    bin.pop();
    if bin.ends_with("deps") {
        bin.pop();
    }
    let out = std::process::Command::new(bin.join("xezim"))
        .env("XEZIM_STALL_LIMIT", "1500")
        .arg(&sv)
        .output()
        .expect("run xezim");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("re-arming via #(p/2)"), "spinner line missing:\n{}", err);
    assert!(
        err.contains("Parked (not spinning)"),
        "parked section missing:\n{}",
        err
    );
    assert!(
        err.contains("next event at time 1"),
        "the parked setter (scheduled at t=1) must be named:\n{}",
        err
    );
}
