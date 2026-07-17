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
