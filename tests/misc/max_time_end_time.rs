//! The closing `Simulation finished at time N` line of a run that
//! `--max-time` stops reports the cap. It used to report the time of the
//! last event that ran, so a design whose only later event lies past the
//! cap said `finished at time 0`, while a free-running clock, which has an
//! event at the cap, said the cap. A run that ends on its own (`$finish`,
//! or nothing left to do) reports its own end time as before.
use std::path::PathBuf;
use std::process::Command;

fn run(tag: &str, src: &str, cap: &str) -> String {
    let dir = std::env::temp_dir().join(format!("xezim_max_end_{}_{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file: PathBuf = dir.join("tb.sv");
    std::fs::write(&file, src).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .current_dir(&dir)
        .args([
            "--no-cache",
            "--simulate",
            "-s",
            "tb",
            "tb.sv",
            "--max-time",
            cap,
        ])
        .output()
        .unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(
        out.status.success(),
        "{tag}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn assert_has(out: &str, want: &str) {
    assert!(
        out.lines().any(|l| l.trim() == want),
        "expected `{want}`, got:\n{out}"
    );
}

/// The issue's case: the only later event is past the cap.
#[test]
fn a_run_stopped_before_a_far_event_reports_the_cap() {
    let out = run(
        "far",
        "module tb; initial begin #2000 $display(\"late\"); $finish; end endmodule\n",
        "1000",
    );
    assert!(!out.contains("late"), "{out}");
    assert_has(&out, "Simulation finished at time 1000");
}

/// A run with an earlier event and a later one past the cap.
#[test]
fn a_run_stopped_after_an_earlier_event_reports_the_cap() {
    let out = run(
        "mid",
        "module tb; initial begin #5 $display(\"early\"); #2000 $finish; end\n\
         final $display(\"final %0t\", $time); endmodule\n",
        "1000",
    );
    assert_has(&out, "early");
    assert_has(&out, "final 1000");
    assert_has(&out, "Simulation finished at time 1000");
}

/// Guard: a free-running clock already reported the cap.
#[test]
fn a_clock_stopped_by_the_cap_still_reports_the_cap() {
    let out = run(
        "clk",
        "module tb; logic c = 0; always #1 c = ~c; endmodule\n",
        "1000",
    );
    assert_has(&out, "Simulation finished at time 1000");
}

/// Guard: runs that end before the cap keep their own end time.
#[test]
fn runs_that_end_before_the_cap_are_unchanged() {
    let idle = run(
        "idle",
        "module tb; initial #5 $display(\"x\"); endmodule\n",
        "1000",
    );
    assert_has(&idle, "Simulation finished at time 5");
    let fin = run("fin", "module tb; initial #10 $finish; endmodule\n", "1000");
    assert_has(&fin, "Simulation finished at time 10 ($finish called)");
}

#[test]
fn an_event_at_the_maximum_timestamp_still_stops_at_the_cap() {
    let out = run(
        "max_timestamp",
        r#"
module tb;
  initial #(64'hffff_ffff_ffff_ffff) $display("late");
  final $display("final %0t", $time);
endmodule
"#,
        "1000",
    );
    assert!(!out.contains("late"), "{out}");
    assert_has(&out, "final 1000");
    assert_has(&out, "Simulation finished at time 1000");
}
