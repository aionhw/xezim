//! Verilog-AMS regression: `wreal` at driver counts a real supply node
//! reaches, driver expressions that are not bare identifiers, and what a
//! `wreal` looks like in a waveform dump.
//!
//! These sit apart from `wreal_nets.rs` (which pins the semantics) because
//! each of them pins a property of the LOWERING rather than of the language:
//! how big the emitted expression gets, that it survives non-trivial driver
//! expressions, and that the resolved net is dumped in the real domain.

use crate::ams_mode::with_ams;
use crate::util::r;
use std::path::PathBuf;
use xezim::simulate;

/// Twelve drivers on one `wrealmin`/`wrealmax` node.
///
/// `?:` cannot bind a temporary, so each fold step duplicates its accumulator
/// into both the condition and one arm. Folded LEFT that doubles the emitted
/// expression per driver — twelve drivers is 4096 nodes and twenty is a
/// million, which elaborates for minutes and then runs slowly forever. The
/// fold is balanced instead, making the size quadratic rather than
/// exponential. This test fails by TIMING OUT (or exhausting memory) if the
/// balancing is ever lost, and by value if the regrouping is ever wrong.
#[test]
fn many_drivers_on_a_minmax_node_stay_tractable() {
    let mut src = String::from("module tb;\n  wrealmin lo;\n  wrealmax hi;\n  real d[0:11];\n");
    for i in 0..12 {
        src.push_str(&format!("  assign lo = d[{i}];\n  assign hi = d[{i}];\n"));
    }
    src.push_str("  initial begin\n");
    // Values 7.0 .. -4.0: the extremes sit in the middle of the list, so a
    // fold that silently dropped a subtree would still look plausible.
    for (i, v) in [7.0, 3.0, -4.0, 5.0, 0.0, 9.0, 2.0, -1.0, 6.0, 4.0, 8.0, 1.0]
        .iter()
        .enumerate()
    {
        src.push_str(&format!("    d[{i}] = {v:.1};\n"));
    }
    src.push_str("    #1;\n  end\nendmodule\n");

    let start = std::time::Instant::now();
    let sim = with_ams(|| simulate(&src, 10).expect("simulate"));
    let elapsed = start.elapsed();

    assert_eq!(r(&sim, "lo"), -4.0);
    assert_eq!(r(&sim, "hi"), 9.0);
    assert!(
        elapsed < std::time::Duration::from_secs(20),
        "12 drivers took {:?} — the min/max fold is not balanced",
        elapsed
    );
}

/// Sixteen drivers on a `wrealsum`, checked against the exact expected total.
/// Addition needs no duplication, but the driver count still has to survive
/// the union-find and the emitted left fold.
#[test]
fn many_drivers_on_a_sum_node_add_up() {
    let mut src = String::from("module tb;\n  wrealsum n;\n  real d[0:15];\n");
    for i in 0..16 {
        src.push_str(&format!("  assign n = d[{i}];\n"));
    }
    src.push_str("  initial begin\n");
    for i in 0..16 {
        src.push_str(&format!("    d[{i}] = {:.1};\n", (i as f64) + 0.5));
    }
    src.push_str("    #1;\n  end\nendmodule\n");

    let sim = with_ams(|| simulate(&src, 10).expect("simulate"));
    // sum(i + 0.5) for i in 0..16 = 120 + 8 = 128
    assert_eq!(r(&sim, "n"), 128.0);
}

/// Drivers that are EXPRESSIONS, not bare identifiers. The reduction embeds
/// each driver's RHS tree directly, and a min/max duplicates it — so a
/// compound driver has to stay correct (and side-effect-free) under
/// re-evaluation.
#[test]
fn compound_driver_expressions_resolve() {
    let sim = with_ams(|| {
        simulate(
            r#"
module tb;
  wrealsum s;
  wrealmax m;
  real a, b;
  assign s = a * 2.0;
  assign s = b + 0.25;
  assign m = a * 2.0;
  assign m = b + 0.25;
  initial begin a = 1.5; b = 4.0; #1; end
endmodule
"#,
            10,
        )
        .expect("simulate")
    });
    // a*2 = 3.0, b+0.25 = 4.25
    assert_eq!(r(&sim, "s"), 7.25);
    assert_eq!(r(&sim, "m"), 4.25);
}

/// A resolved `wreal` reaches a VCD dump as a REAL (`$var real 64`, `r<dec>`
/// value changes), not as a 64-bit vector of the float's raw bit image. The
/// net is synthesized by the resolution fold, so it is worth confirming it
/// carries the real flag all the way to the sink.
#[test]
fn a_resolved_wreal_dumps_as_a_real() {
    let mut path: PathBuf = std::env::temp_dir();
    path.push(format!("xezim_ams_wreal_{}.vcd", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let src = format!(
        r#"
module tb;
  wrealsum n;
  real a, b;
  assign n = a;
  assign n = b;
  initial begin
    $dumpfile("{}"); $dumpvars(0, tb);
    a = 1.5; b = 2.25;
    #1;
    a = 3.0;
    #1;
  end
endmodule
"#,
        path.to_str().unwrap()
    );
    with_ams(|| simulate(&src, 100).expect("simulate"));
    let vcd = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("no VCD written to {}: {}", path.display(), e));
    let _ = std::fs::remove_file(&path);

    let var = vcd
        .lines()
        .find(|l| l.starts_with("$var") && l.split_whitespace().nth(4) == Some("n"))
        .unwrap_or_else(|| panic!("no $var for `n` in:\n{}", vcd));
    assert!(
        var.contains("real"),
        "a wreal must be declared `$var real`, got: {}",
        var
    );
    assert!(
        vcd.lines().any(|l| l.starts_with("r3.75")),
        "expected the resolved 3.75 as an `r<decimal>` change in:\n{}",
        vcd
    );
    assert!(
        vcd.lines().any(|l| l.starts_with("r5.25")),
        "expected the re-resolved 5.25 after `a` changes in:\n{}",
        vcd
    );
}

/// The resolution is re-evaluated when a driver changes — a `wreal` node is a
/// continuous assignment, not a one-shot fold at time 0.
#[test]
fn the_resolution_tracks_driver_changes() {
    let sim = with_ams(|| {
        simulate(
            r#"
module tb;
  wrealsum n;
  real a, b, seen_t1, seen_t2;
  assign n = a;
  assign n = b;
  initial begin
    a = 1.0; b = 2.0;
    #1 seen_t1 = n;
    a = 10.0;
    #1 seen_t2 = n;
  end
endmodule
"#,
            100,
        )
        .expect("simulate")
    });
    assert_eq!(r(&sim, "seen_t1"), 3.0);
    assert_eq!(r(&sim, "seen_t2"), 12.0, "the fold must re-evaluate");
}
