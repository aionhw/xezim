//! §10.3, §4.4 (#1149): a blocking write by a process defers the continuous
//! assignments it drives until the process yields. A long time-0 block holds
//! that list until its start batch ends, and an assignment that re-evaluates
//! at every settle (one reading a static class property) was deferred again,
//! with its reads appended again, on every blocking write: the list grew
//! without bound. It now holds each signal once; the order of the updates is
//! unchanged.

use std::path::PathBuf;
use std::process::Command;

fn xezim_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_xezim"))
}

fn run(name: &str, source: &str, env: &[(&str, &str)]) -> (String, String) {
    let dir = std::env::temp_dir().join(format!(
        "xezim_deferred_assign_reads_1149_{}_{}",
        name,
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create temp directory");
    let path = dir.join(format!("{name}.sv"));
    std::fs::write(&path, source).expect("write test source");
    let mut cmd = Command::new(xezim_bin());
    cmd.arg("--no-cache").arg(&path).current_dir(&dir);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run xezim");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(out.status.success(), "xezim failed: {out:?}");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// The `deferred_entries=` values of the `[PSETTLE]` reports (one every
/// 20000 procedural writes).
fn deferred_entries(stderr: &str) -> Vec<u64> {
    stderr
        .lines()
        .filter(|l| l.starts_with("[PSETTLE]"))
        .filter_map(|l| {
            l.split_whitespace()
                .find_map(|f| f.strip_prefix("deferred_entries="))
                .and_then(|v| v.parse().ok())
        })
        .collect()
}

fn checks(stdout: &str) -> Vec<&str> {
    stdout
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("T|"))
        .collect()
}

const LONG_TIME_ZERO_BLOCK: &str = r#"
module top;
  logic [7:0] sel;
  wire  [7:0] sel_next = sel + 8'd1;
  logic [31:0] r0 = 0, r1 = 0, r2 = 0, r3 = 0;
  class cfg_t;
    static int level;
    int hits;
  endclass
  // Reads a static class property, so it is evaluated at every settle.
  wire [31:0] mix = cfg_t::level + r0 + r1 + r2 + r3;
  class item_t;
    int id;
    item_t link;
  endclass
  item_t head, it;
  int made;
  initial begin
    sel = 8'd1;
    for (int i = 0; i < 30000; i++) begin
      it = new;
      it.id = i;
      it.link = head;
      head = it;
      made++;
    end
    $display("T|built %0d sel_next=%0d", made, sel_next);
    #1;
    r2 = 32'd5;
    sel = 8'd7;
    #1;
    $display("T|after sel_next=%0d mix=%0d", sel_next, mix);
    $finish;
  end
endmodule
"#;

#[test]
fn deferred_reads_stay_bounded_in_a_long_time_zero_block() {
    let (stdout, stderr) = run(
        "bounded",
        LONG_TIME_ZERO_BLOCK,
        &[("XEZIM_PSETTLE_STATS", "1")],
    );
    assert_eq!(
        checks(&stdout),
        ["T|built 30000 sel_next=x", "T|after sel_next=8 mix=5"],
        "{stdout}"
    );
    let lens = deferred_entries(&stderr);
    assert!(
        lens.len() >= 3,
        "expected [PSETTLE] reports, got:\n{stderr}"
    );
    // The list holds each read signal once: sel, r0..r3 and the marker.
    assert!(
        lens.iter().all(|&n| n <= 8),
        "the deferred cont-assign reads grew with the writes of the block: {lens:?}"
    );
}

#[test]
fn deferred_updates_keep_their_order_after_the_block() {
    let (stdout, _) = run(
        "order",
        r#"
module top;
  logic [7:0] sel;
  wire  [7:0] sel_next = sel + 8'd1;
  class cfg_t;
    static int level;
  endclass
  wire [31:0] level_mirror = cfg_t::level;
  class item_t;
    int id;
    item_t link;
  endclass
  item_t head, it;
  int made;
  initial begin
    sel = 8'd1;
    for (int i = 0; i < 3000; i++) begin
      it = new;
      it.id = i;
      it.link = head;
      head = it;
      made++;
    end
    $display("T|in block sel_next=%0d made=%0d", sel_next, made);
    #1;
    $display("T|after yield sel_next=%0d", sel_next);
    sel = 8'd7;
    $display("T|before yield sel_next=%0d", sel_next);
    #1;
    $display("T|end sel_next=%0d level_mirror=%0d", sel_next, level_mirror);
    $finish;
  end
  always @(sel_next) $display("T|observer sel_next=%0d at %0t", sel_next, $time);
endmodule
"#,
        &[],
    );
    assert_eq!(
        checks(&stdout),
        [
            "T|in block sel_next=x made=3000",
            "T|observer sel_next=2 at 0",
            "T|after yield sel_next=2",
            "T|before yield sel_next=2",
            "T|observer sel_next=8 at 1",
            "T|end sel_next=8 level_mirror=0",
        ],
        "{stdout}"
    );
}

#[test]
fn unresolved_assignment_with_signal_reads_runs_after_yield() {
    let (stdout, _) = run(
        "unresolved_reads",
        r#"
module top;
  class cfg;
    static int offset;
  endclass
  logic [7:0] a = 0, b = 0;
  wire [7:0] sum = a + b + cfg::offset;
  initial begin
    #1;
    a = 7;
    b = 8;
    $display("T|before yield %0d", sum);
    #1;
    $display("T|after yield %0d", sum);
    $finish;
  end
endmodule
"#,
        &[],
    );
    assert_eq!(checks(&stdout), ["T|before yield 0", "T|after yield 15"]);
}
