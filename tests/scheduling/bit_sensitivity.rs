//! Bit-granular combinational sensitivity.
//!
//! A write inside the settle loop re-runs only the combinational readers of
//! the bits it changed: `assign lo = bus[3:0]` stays idle while `bus[7:4]`
//! moves. The stimulus comes from clocked blocks, so the logic settles
//! outside any process (where the two-state fast path, and with it the
//! masks, applies). Every case checks its values against the expressions
//! on the opposite clock edge (a stale reader prints MISMATCH), prints the
//! same lines with the masks off (`XEZIM_BIT_SENS=0`), and — where the
//! shape allows — shows the masks saved evaluations.
use std::path::PathBuf;
use std::process::Command;

fn run(name: &str, src: &str, bit_sens: bool) -> String {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("bit_sensitivity");
    std::fs::create_dir_all(&dir).unwrap();
    let sv = dir.join(format!("{name}.sv"));
    std::fs::write(&sv, src).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .env("XEZIM_BIT_SENS", if bit_sens { "1" } else { "0" })
        // `--verbose`: the "[PROF] ... entry_evals=" counter read below.
        .args([
            "--simulate",
            "--verbose",
            "-s",
            "tb",
            "--no-cache",
            sv.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    assert!(output.status.success(), "run failed:\n{text}");
    text
}

/// The simulation's own lines (checks and results), in order.
fn sim_lines(text: &str) -> Vec<&str> {
    text.lines()
        .filter(|l| l.starts_with("OK ") || l.starts_with("MISMATCH") || l.starts_with("RES "))
        .collect()
}

/// Combinational evaluations the settle loop ran.
fn evals(text: &str) -> u64 {
    let line = text
        .lines()
        .find(|l| l.contains("entry_evals="))
        .unwrap_or_else(|| panic!("no entry_evals line:\n{text}"));
    line.split("entry_evals=")
        .nth(1)
        .unwrap()
        .split(' ')
        .next()
        .unwrap()
        .parse()
        .unwrap()
}

/// Runs `src` both ways; returns (lines, evaluations with masks on, off).
fn both(name: &str, src: &str, ok_lines: usize) -> (Vec<String>, u64, u64) {
    let on = run(name, src, true);
    let off = run(&format!("{name}_off"), src, false);
    let a = sim_lines(&on);
    let b = sim_lines(&off);
    assert!(
        !a.iter().any(|l| l.starts_with("MISMATCH")),
        "stale reader with masks on:\n{on}"
    );
    assert!(
        !b.iter().any(|l| l.starts_with("MISMATCH")),
        "stale reader with masks off:\n{off}"
    );
    assert_eq!(a, b, "masks changed the simulation");
    assert_eq!(
        a.iter().filter(|l| l.starts_with("OK ")).count(),
        ok_lines,
        "missing checks:\n{on}"
    );
    (
        a.iter().map(|s| s.to_string()).collect(),
        evals(&on),
        evals(&off),
    )
}

/// §9.4.2 / §10.4.2: narrower write arming must preserve sampled slice
/// values and NBA updates. Exercise disjoint slices, a union of two reads,
/// whole and dynamic reads, and transitions in each of the two logic planes.
#[test]
fn clocked_slice_readers_arm_on_value_and_xz_changes() {
    let src = r#"
`timescale 1ns/1ps
module tb;
  reg clk = 0;
  always #5 clk = ~clk;
  reg [15:0] bus = 0;
  reg [95:0] wide = 0;
  reg [3:0] index = 0;
  reg [3:0] low, high, dyn, wide_low, wide_high;
  reg [1:0] mix;
  reg [15:0] all_bits;
  always @(posedge clk) low <= bus[3:0];
  always @(posedge clk) high <= bus[11:8];
  always @(posedge clk) mix <= {bus[8], bus[0]};
  always @(posedge clk) all_bits <= bus;
  always @(posedge clk) dyn <= bus[index +: 4];
  always @(posedge clk) wide_low <= wide[5:2];
  always @(posedge clk) wide_high <= wide[81:78];

  always @(negedge clk) begin
    if (low !== bus[3:0] || high !== bus[11:8] ||
        mix !== {bus[8], bus[0]} || all_bits !== bus ||
        dyn !== bus[index +: 4] || wide_low !== wide[5:2] ||
        wide_high !== wide[81:78])
      $fatal(1, "stale clocked value at %0t", $time);
    $display("OK %b %b %b %b %b %b %b", low, high, mix, all_bits, dyn, wide_low, wide_high);
  end

  initial begin
    for (integer phase = 0; phase < 4; phase++) begin
      repeat (4) begin
        @(negedge clk);
        #1;
        bus[11:8] <= bus[11:8] + 1'b1;
        wide[5:2] <= wide[5:2] + 1'b1;
      end
      case (phase)
        0: begin bus[0] = 1'b1; wide[79] = 1'b1; end
        1: begin bus[0] = 1'bz; wide[79] = 1'bz; end
        2: begin bus[0] = 1'bx; wide[79] = 1'bx; end
        3: begin bus[0] = 1'b0; wide[79] = 1'b0; end
      endcase
      index = phase[0] ? 8 : 0;
    end
    #20 $finish;
  end
endmodule
"#;
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("bit_sensitivity");
    std::fs::create_dir_all(&dir).unwrap();
    let sv = dir.join("clocked_slice_readers.sv");
    std::fs::write(&sv, src).unwrap();
    let run = |masks, shadow, args: &[&str]| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_xezim"));
        command
            .env("XEZIM_ARMED_EDGE", "1")
            .env("XEZIM_EVENT_EDGE", "1")
            .env("XEZIM_EDGE_MERGE", "0")
            .env("XEZIM_ARMED_EDGE_MASKS", masks)
            .env_remove("XEZIM_ARMED_EDGE_SHADOW")
            .args(["--no-cache", "--error-exit", "--verbose", "-s", "tb"])
            .args(args)
            .arg(&sv);
        if shadow {
            command.env("XEZIM_ARMED_EDGE_SHADOW", "1");
        }
        let out = command.output().unwrap();
        let text = format!(
            "{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(out.status.success(), "{text}");
        text
    };
    let off = run("0", false, &[]);
    let on = run("1", false, &[]);
    let shadow = run("1", true, &[]);
    assert_eq!(sim_lines(&on).len(), 18, "{on}");
    assert_eq!(sim_lines(&on), sim_lines(&off));
    assert_eq!(sim_lines(&shadow), sim_lines(&off));
    let counter = |text: &str, marker: &str| -> u64 {
        text.lines()
            .find_map(|line| line.split_once(marker))
            .unwrap_or_else(|| panic!("missing {marker}:\n{text}"))
            .1
            .split_whitespace()
            .next()
            .unwrap()
            .parse()
            .unwrap()
    };
    assert!(counter(&on, "bit-selective arm fanouts=") >= 2, "{on}");
    assert!(counter(&on, "armed-fast-skips=") > counter(&off, "armed-fast-skips="));

    // Save with a data write pending for the next posedge. The restored
    // run needs both the saved arm flags and the filtered fanout for
    // the writes made after resuming.
    let checkpoint = dir.join("clocked_slice_readers.ckpt");
    let save_at = format!("91ns,{}", checkpoint.display());
    let saved = run("1", false, &["--save-at", &save_at]);
    assert_eq!(sim_lines(&saved), sim_lines(&off));
    let restored = run("1", false, &["--restore", checkpoint.to_str().unwrap()]);
    assert_eq!(sim_lines(&restored), sim_lines(&off)[9..], "{restored}");
}

/// Field readers of a bus written by a lowered continuous assignment, a
/// one-bit gate on two of its bits, and a whole-bus reader.
#[test]
fn slice_readers_of_a_computed_bus() {
    let src = r#"
module tb;
  reg clk = 0;
  reg [7:0] a = 0, b = 0;
  integer i = 0;
  always @(posedge clk) begin
    i <= i + 1;
    if (i % 3 == 0) a <= a + 8'd3; else b <= b + 8'd5;
  end
  wire [15:0] bus;
  assign bus = {a ^ 8'h0f, b + 8'd1};
  wire [3:0] q0 = bus[3:0];
  wire [3:0] q1 = bus[7:4];
  wire [7:0] q2 = bus[15:8];
  wire p = bus[9] & bus[2];
  wire [15:0] r = bus;
  always @(negedge clk) begin
    if (q0 !== bus[3:0] || q1 !== bus[7:4] || q2 !== bus[15:8]
        || p !== (bus[9] & bus[2]) || r !== bus)
      $display("MISMATCH %0t %h %h %h %h %b %h", $time, bus, q0, q1, q2, p, r);
    $display("OK %0t bus=%h q0=%h q1=%h q2=%h p=%b", $time, bus, q0, q1, q2, p);
  end
  initial begin
    repeat (80) #5 clk = ~clk;
    $finish;
  end
endmodule
"#;
    let (_, on, off) = both("slice_readers", src, 39);
    assert!(on < off, "no evaluation saved: {on} vs {off}");
}

/// Two assignments drive the halves of one bus and run back to back. The
/// second one's store must still trigger the readers of ITS half: a store
/// may share a dirty record only with an earlier store of the same entry,
/// never with the record the previous entry already propagated.
#[test]
fn two_drivers_of_one_bus() {
    let src = r#"
module tb;
  reg clk = 0;
  reg [3:0] x = 0, y = 0;
  integer i = 0;
  always @(posedge clk) begin
    i <= i + 1;
    x <= x + 4'd1;
    if (i % 2 == 0) y <= y + 4'd3;
  end
  wire [7:0] m;
  assign m[3:0] = x + 4'd1;
  assign m[7:4] = y ^ 4'ha;
  wire [3:0] m_lo = m[3:0];
  wire [3:0] m_hi = m[7:4];
  wire [7:0] m_all = m;
  always @(negedge clk) begin
    if (m_lo !== x + 4'd1 || m_hi !== (y ^ 4'ha) || m_all !== {y ^ 4'ha, x + 4'd1})
      $display("MISMATCH %0t %h %h %h", $time, m_lo, m_hi, m_all);
    $display("OK %0t m=%h lo=%h hi=%h", $time, m_all, m_lo, m_hi);
  end
  initial begin
    repeat (60) #5 clk = ~clk;
    $finish;
  end
endmodule
"#;
    both("two_drivers", src, 29);
}

/// A 200-bit bus maps four signal bits to one mask bit: a store to bits
/// 130..120 must reach a reader of bit 131 (same chunk as 128..130) and a
/// reader straddling 135..128, while a reader of 119..118 may stay idle.
#[test]
fn wide_bus_chunks() {
    let src = r#"
module tb;
  reg clk = 0;
  reg [10:0] c = 0;
  reg [199:0] wbase = {200{1'b1}};
  integer i = 0;
  always @(posedge clk) begin
    i <= i + 1;
    if (i % 4 == 3) wbase <= {wbase[198:0], wbase[199] ^ wbase[3]};
    else c <= c + 11'd7;
  end
  wire [199:0] wb;
  assign wb[119:0] = wbase[119:0];
  assign wb[130:120] = c;
  assign wb[199:131] = wbase[199:131];
  wire [3:0] r1 = wb[123:120];
  wire [1:0] r2 = wb[119:118];
  wire r3 = wb[131];
  wire [7:0] r4 = wb[135:128];
  wire [199:0] rall = wb;
  always @(negedge clk) begin
    if (r1 !== c[3:0] || r2 !== wbase[119:118] || r3 !== wbase[131]
        || r4 !== {wbase[135:131], c[10:8]}
        || rall !== {wbase[199:131], c, wbase[119:0]})
      $display("MISMATCH %0t %h %h %b %h", $time, r1, r2, r3, r4);
    $display("OK %0t c=%h r1=%h r2=%h r3=%b r4=%h", $time, c, r1, r2, r3, r4);
  end
  initial begin
    repeat (60) #5 clk = ~clk;
    $finish;
  end
endmodule
"#;
    both("wide_bus", src, 29);
}

/// Blocks the masks must leave whole-signal: a block that reads what it
/// writes (`cnt = cnt + s[0]` carries state, so every change of `s`
/// counts), an `always @*` that stores its output twice, and an explicit
/// list that reads a signal it is not sensitive to (`y1` must pick up the
/// current `z` whenever any bit of `s` moves). Beside them, an `always @*`
/// reading one bit of `s`.
#[test]
fn stateful_and_partial_sensitivity_blocks() {
    let src = r#"
module tb;
  reg clk = 0;
  integer i = 0;
  reg [7:0] a = 0;
  reg b0 = 1;
  reg [3:0] z = 4'h5;
  always @(posedge clk) begin
    i <= i + 1;
    a <= a + 8'd2;
    if (i % 5 == 0) b0 <= ~b0;
    if (i % 7 == 0) z <= z + 4'd1;
  end
  wire [7:0] s = {a[6:0] ^ {3'b0, z}, b0};
  integer cnt = 0;
  always @(s) cnt = cnt + s[0];
  reg [3:0] y1, o2, y3;
  always @(s) y1 = {3'b0, s[0]} ^ z;
  always @* begin o2 = 0; if (s[4]) o2 = s[7:5]; end
  always @* y3 = {3'b0, s[1]} ^ z;
  always @(negedge clk) begin
    if (o2 !== (s[4] ? {1'b0, s[7:5]} : 4'd0) || y3 !== ({3'b0, s[1]} ^ z))
      $display("MISMATCH %0t %h %h", $time, o2, y3);
    $display("OK %0t s=%h cnt=%0d y1=%h o2=%h y3=%h", $time, s, cnt, y1, o2, y3);
  end
  initial begin
    repeat (80) #5 clk = ~clk;
    $display("RES cnt=%0d", cnt);
    $finish;
  end
endmodule
"#;
    let (lines, _, _) = both("stateful", src, 39);
    assert!(lines.iter().any(|l| l.starts_with("RES cnt=")), "{lines:?}");
}

/// A plain `always` block's variable written by another process: the block
/// must keep re-running on every change of what it is sensitive to, so it
/// overwrites the foreign value exactly when it always did (`q` returns to
/// `a[0]` when `a[1]` moves; with a bit mask it kept the foreign value in 11
/// of the 29 checks), and the `always_comb` beside it still skips changes of
/// bits it does not read.
#[test]
fn plain_always_output_with_another_writer() {
    let src = r#"
module tb;
  reg clk = 0;
  integer i = 0;
  reg [7:0] cnt = 0;
  // Comb-driven, so its changes reach the readers through the settle
  // loop, where the masks apply.
  wire [7:0] a;
  assign a = cnt ^ 8'h00;
  reg q;
  logic r;
  always @* q = a[0];
  always_comb r = a[0] ^ a[7];
  always @(posedge clk) begin
    i <= i + 1;
    cnt <= cnt + 8'd2;
    if (i % 4 == 1) cnt[0] <= ~cnt[0];
  end
  always @(negedge clk) begin
    $display("OK %0t a=%h q=%b r=%b", $time, a, q, r);
    if (i % 5 == 2) q = ~q;
  end
  initial begin
    repeat (60) #5 clk = ~clk;
    $finish;
  end
endmodule
"#;
    both("plain_always_other_writer", src, 29);
}
