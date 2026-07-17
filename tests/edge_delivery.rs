//! Event-delivery correctness in the scheduler core.
//!
//! §4.4.2.3: a `#0` continuation resumes in the Inactive region of the SAME
//! timestamp. A blocking write it performs is an ordinary active-region event
//! of a later delta cycle — an `@(posedge/negedge/anyedge)` waiter registered
//! in an earlier delta cycle of that timestamp must see the resulting edge.
//! xezim used to skip every waiter registered at the current timestamp
//! wholesale, so an edge produced by an inactive-region write was never
//! delivered and the waiter hung forever (Icarus fires it at the same time).
//!
//! §9.2: every change on a signal in an `always @(...)` sensitivity list must
//! (re-)trigger the block — including a change made by another edge-triggered
//! block within the same delta batch. xezim used to coalesce those away
//! (silent event loss); correct delivery turns a two-block ping-pong into a
//! genuine zero-delay livelock, which must then hit the stall detector and
//! terminate with an attributed report instead of hanging.

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

fn find_line<'a>(sim: &'a xezim::compiler::Simulator, prefix: &str) -> Option<&'a str> {
    sim.output
        .iter()
        .map(|o| o.message.as_str())
        .find(|m| m.starts_with(prefix))
}

/// S1 shape (a): the edge is produced at time 0 by a `#0` (Inactive-region)
/// blocking assign. The waiter registered in the first delta cycle of time 0
/// and must still be woken — Icarus prints `hits=1 t=0`.
#[test]
fn posedge_from_inactive_region_write_at_time_zero_wakes_waiter() {
    const SRC: &str = r#"
`timescale 1ps/1ps
module t; reg clk = 0; int hits = 0;
  initial begin #0 clk = 1; end
  initial begin @(posedge clk) hits++; $display("HIT hits=%0d t=%0t", hits, $time); $finish; end
  initial #1000 begin $display("MISSED"); $finish; end
endmodule
"#;
    let sim = run(SRC, &[]);
    assert!(
        find_line(&sim, "HIT hits=1").is_some(),
        "the t=0 inactive-region posedge must wake the waiter, got: {:?}",
        sim.output.iter().map(|o| &o.message).collect::<Vec<_>>()
    );
    assert!(find_line(&sim, "MISSED").is_none(), "watchdog fired: edge was lost");
    assert_eq!(sim.time, 0, "the waiter must fire at time 0, not at the watchdog");
}

/// S1 shape (b): same class at a NONZERO time — `#5; #0 clk = 1;`.
#[test]
fn posedge_from_inactive_region_write_at_nonzero_time_wakes_waiter() {
    const SRC: &str = r#"
`timescale 1ps/1ps
module t; reg clk = 0; int hits = 0;
  initial begin #5; #0 clk = 1; end
  initial begin @(posedge clk) hits++; $display("HIT hits=%0d t=%0t", hits, $time); $finish; end
  initial #1000 begin $display("MISSED"); $finish; end
endmodule
"#;
    let sim = run(SRC, &[]);
    assert!(
        find_line(&sim, "HIT hits=1").is_some(),
        "the t=5 inactive-region posedge must wake the waiter, got: {:?}",
        sim.output.iter().map(|o| &o.message).collect::<Vec<_>>()
    );
    assert_eq!(sim.time, 5, "the waiter must fire at time 5");
}

/// S1 shape (c): NEGEDGE variant — 1 -> 0 through the inactive region.
#[test]
fn negedge_from_inactive_region_write_wakes_waiter() {
    const SRC: &str = r#"
`timescale 1ps/1ps
module t; reg clk = 1; int hits = 0;
  initial begin #0 clk = 0; end
  initial begin @(negedge clk) hits++; $display("HIT hits=%0d t=%0t", hits, $time); $finish; end
  initial #1000 begin $display("MISSED"); $finish; end
endmodule
"#;
    let sim = run(SRC, &[]);
    assert!(
        find_line(&sim, "HIT hits=1").is_some(),
        "the t=0 inactive-region negedge must wake the waiter, got: {:?}",
        sim.output.iter().map(|o| &o.message).collect::<Vec<_>>()
    );
    assert!(find_line(&sim, "MISSED").is_none(), "watchdog fired: edge was lost");
    assert_eq!(sim.time, 0);
}

/// The time-0 init pseudo-edge protection must survive the S1 fix: a waiter
/// registered at time 0 must NOT fire on the initializations that seeded the
/// signal's value at elaboration (prev = X so everything "changes" at the
/// first check). `clk` is initialized to 1 and never toggles; `@(posedge
/// clk)` must wait forever (watchdog ends the sim).
#[test]
fn time_zero_init_values_do_not_spuriously_wake_waiters() {
    const SRC: &str = r#"
`timescale 1ps/1ps
module t; reg clk = 1; int hits = 0;
  initial begin @(posedge clk) hits++; $display("SPURIOUS hits=%0d", hits); $finish; end
  initial #100 begin $display("CLEAN"); $finish; end
endmodule
"#;
    let sim = run(SRC, &[]);
    assert!(
        find_line(&sim, "SPURIOUS").is_none(),
        "init value must not read as a posedge for a t=0 waiter"
    );
    assert!(find_line(&sim, "CLEAN").is_some());
}

/// S2 safety net: with §9.2 re-triggering delivered correctly, a two-block
/// signal ping-pong (`always @(a) b=~b;` / `always @(b) a=~a;`) is a genuine
/// zero-delay livelock (Icarus spins forever on it). It must terminate via
/// the stall detector with a report NAMING both always blocks — not hang,
/// and not silently "survive" to a later time with the events dropped.
#[test]
fn signal_ping_pong_livelocks_into_attributed_stall() {
    use std::process::Command;

    fn xezim_bin() -> std::path::PathBuf {
        let mut p = std::env::current_exe().expect("current_exe");
        p.pop();
        if p.ends_with("deps") {
            p.pop();
        }
        p.join("xezim")
    }

    let dir = std::env::temp_dir().join("xezim_edge_pingpong_test");
    std::fs::create_dir_all(&dir).expect("mkdir");
    let sv = dir.join("pingpong.sv");
    // The two always blocks sit on lines 2 and 3 of the file.
    std::fs::write(
        &sv,
        "module t; reg a = 0, b = 0; int n = 0;\n  always @(a) begin n++; b = ~b; end\n  always @(b) begin n++; a = ~a; end\n  initial begin #0 a = 1; end\n  initial #10 begin $display(\"SURVIVED n=%0d\", n); $finish; end\nendmodule\n",
    )
    .expect("write sv");
    let out = Command::new(xezim_bin())
        .env("XEZIM_STALL_LIMIT", "2000")
        .args(["--simulate", "-s", "t"])
        .arg(&sv)
        .output()
        .expect("run xezim");
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stderr.contains("simulation STALLED"),
        "the ping-pong must trip the stall detector:\nstderr:{}\nstdout:{}",
        stderr,
        stdout
    );
    assert!(
        !stdout.contains("SURVIVED"),
        "the sim must NOT coast to t=10 with the re-triggers dropped:\n{}",
        stdout
    );
    // Both blocks must be named, with file:line and their sensitivity.
    for (line, sens) in [(2, "@(a)"), (3, "@(b)")] {
        assert!(
            stderr.contains(&format!("always block at {}:{}", sv.display(), line)),
            "stall report must name the always block on line {}:\n{}",
            line,
            stderr
        );
        assert!(
            stderr.contains(&format!("sensitive to {}", sens)),
            "stall report must show the block's sensitivity {}:\n{}",
            sens,
            stderr
        );
    }
    assert!(
        stderr.contains("ran 2000 times at this timestamp"),
        "the count phrasing must carry the per-block execution count:\n{}",
        stderr
    );
}

/// S2 correctness: a LEGITIMATE blocking-write cascade a -> b -> c through
/// `always @` blocks must deliver every re-trigger AND terminate. Before the
/// fix, a change made by an edge block during the same delta batch was
/// coalesced away: block 2 never saw b change, c stayed 0 (silent event
/// loss in handshake-style logic).
#[test]
fn blocking_write_cascade_retriggers_each_stage_and_terminates() {
    const SRC: &str = r#"
module t; reg a = 0, b = 0, c = 0; int na = 0, nb = 0, nc = 0;
  always @(a) begin na++; b = a; end
  always @(b) begin nb++; c = b; end
  always @(c) begin nc++; end
  initial begin #0 a = 1; end
  initial #10 begin $display("CHAIN na=%0d nb=%0d nc=%0d c=%b", na, nb, nc, c); $finish; end
endmodule
"#;
    let sim = run(SRC, &[]);
    assert_eq!(sim.time, 10, "the cascade must settle and let time advance");
    let line = find_line(&sim, "CHAIN ").expect("no CHAIN line").to_string();
    // Each stage runs once at t=0 (xezim's init-detect fires every block
    // once against the X->init pseudo-edge) and exactly once more for the
    // #0-driven wave — the b and c re-triggers are the part the old
    // coalescing dropped. Exact counts also prove no double-delivery.
    assert_eq!(
        line.trim(),
        "CHAIN na=2 nb=2 nc=2 c=1",
        "every stage of the cascade must re-trigger exactly once per change"
    );
}

/// S3d: the settle-limit warning must NAME the non-converging signals —
/// value, and the driving block's file:line — not just say "signals may not
/// have converged". Asserted through the CLI binary (the warning goes to
/// stderr) — same subprocess pattern as tests/determinism_and_stall.rs.
#[test]
fn settle_limit_warning_names_the_oscillating_signals() {
    use std::process::Command;

    fn xezim_bin() -> std::path::PathBuf {
        let mut p = std::env::current_exe().expect("current_exe");
        p.pop();
        if p.ends_with("deps") {
            p.pop();
        }
        p.join("xezim")
    }

    let dir = std::env::temp_dir().join("xezim_settle_warn_test");
    std::fs::create_dir_all(&dir).expect("mkdir");
    let sv = dir.join("settle_ring.sv");
    // A zero-delay combinational oscillator: a -> b -> a with an inverter.
    // The always_comb blocks sit on lines 3 and 4 of the file.
    std::fs::write(
        &sv,
        "module t;\n  logic a, b;\n  always_comb a = ~b;\n  always_comb b = a;\n  initial begin a = 0; #10 $finish; end\nendmodule\n",
    )
    .expect("write sv");
    let out = Command::new(xezim_bin()).arg(&sv).output().expect("run xezim");
    let stderr = String::from_utf8_lossy(&out.stderr);
    // The established first line must survive verbatim (scripts grep it).
    assert!(
        stderr.contains("settle limit hit (100 iters) at time 0 — signals may not have converged"),
        "settle-limit warning missing or first line changed:\n{}",
        stderr
    );
    // ... and it must now carry attribution: both ring signals with values
    // and the driving block's file:line.
    assert!(
        stderr.contains("Still changing in the last"),
        "attribution section missing:\n{}",
        stderr
    );
    for (sig, line) in [("a = 'b", 3), ("b = 'b", 4)] {
        assert!(
            stderr.contains(sig),
            "signal + value missing from settle warning:\n{}",
            stderr
        );
        assert!(
            stderr.contains(&format!("always_comb block at {}:{}", sv.display(), line)),
            "driver file:line missing from settle warning:\n{}",
            stderr
        );
    }
}

/// A `forever @(posedge clk)` loop must fire exactly once per edge — the
/// re-registration after each wake must not re-consume the same edge in a
/// later delta cycle of the same timestamp.
#[test]
fn forever_at_posedge_fires_exactly_once_per_edge() {
    const SRC: &str = r#"
`timescale 1ps/1ps
module t; reg clk = 0; int hits = 0;
  always #5 clk = ~clk;
  initial forever @(posedge clk) hits++;
  initial #52 begin $display("HITS %0d", hits); $finish; end
endmodule
"#;
    let sim = run(SRC, &[]);
    // Posedges at t=10,20,30,40,50 -> exactly 5.
    let line = find_line(&sim, "HITS ").expect("no HITS line").to_string();
    assert_eq!(line.trim(), "HITS 5", "one wake per posedge, got: {}", line);
}

/// The NBA-feedback delta loop (`always @(fb) fb <= ~fb;` — the classic PLL
/// behavioral model) used to be silently DROPPED after the cascade limit: the
/// pending non-blocking writes were discarded and the sim continued with stale
/// state. It must instead stop with an attributed stall report naming the
/// feedback signal.
#[test]
fn nba_feedback_loop_is_reported_not_dropped() {
    let dir = std::env::temp_dir().join("xezim_nba_loop");
    std::fs::create_dir_all(&dir).unwrap();
    let sv = dir.join("nba_loop.sv");
    std::fs::write(
        &sv,
        "module pll_cell(); reg fb = 0; always @(fb) fb <= ~fb; endmodule\n\
         module tb; pll_cell u_pll(); initial #100 $finish; endmodule\n",
    )
    .unwrap();
    let mut bin = std::env::current_exe().unwrap();
    bin.pop();
    if bin.ends_with("deps") { bin.pop(); }
    let out = std::process::Command::new(bin.join("xezim"))
        .arg(&sv)
        .output()
        .expect("run");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("did not converge") && err.contains("u_pll.fb"),
        "NBA loop must be reported with the feedback signal named:\n{}",
        err
    );
}

/// §9.2.1: `always` with no timing control anywhere is a COMPILE error (it can
/// never yield). It used to be classified as combinational with self-written
/// vars dropped from sensitivity — ran once, silently.
#[test]
fn always_without_timing_control_is_a_compile_error() {
    let bad = "module t; reg fb = 0; always fb = ~fb; endmodule";
    assert!(
        xezim::simulate(bad, 1000).is_err(),
        "timing-control-less always must be rejected"
    );
    // The legal neighbors must stay accepted.
    for good in [
        "module t; reg c = 0; always #10 c = ~c; endmodule",
        "module t; reg c = 0, q; always @(posedge c) q <= 1; endmodule",
        "module t; task automatic tk; #5; endtask always tk(); endmodule",
    ] {
        assert!(
            xezim::simulate(good, 100).is_ok(),
            "legal always form over-rejected: {}",
            good
        );
    }
}
