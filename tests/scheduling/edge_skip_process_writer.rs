//! An edge block whose output a PROCESS also writes must not skip an event.
//!
//! The event-edge engine skips a fire when none of the block's data inputs
//! changed since its last one, which is sound only while the block alone
//! determines its outputs. Other edge blocks and settle entries were counted
//! as drivers; initial blocks, always blocks run as processes and the tasks
//! they call were not. `always @(a) ext = 1;` reads no data input at all, so
//! after the testbench cleared `ext` every later event of `a` was skipped and
//! `ext` stayed 0; a flop whose `q` the testbench overwrote between edges
//! kept the overwritten value. IEEE 1800-2017 §9.4.2: an event control
//! resumes its process on every change of a listed expression (every
//! matching edge for `posedge`), and §9.2.2.1 re-executes the statement each
//! time. Expected values were checked against the reference simulator.
use std::path::PathBuf;
use std::process::Command;

fn run(name: &str, src: &str) -> String {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("edge_skip_process_writer");
    std::fs::create_dir_all(&dir).unwrap();
    let sv = dir.join(format!("{name}.sv"));
    std::fs::write(&sv, src).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--simulate", "-s", "tb", "--no-cache", sv.to_str().unwrap()])
        .output()
        .unwrap();
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    assert!(output.status.success(), "run failed:\n{text}");
    text
}

fn lines_with<'a>(text: &'a str, key: &str) -> Vec<&'a str> {
    text.lines().filter(|l| l.contains(key)).collect()
}

/// The reported shape: a waiter woken by an always block whose single
/// assignment does not read the event signal. Every change of `a` (including
/// 1 -> z) must wake it again.
#[test]
fn always_event_body_not_reading_event_signal_refires() {
    let src = r#"
`timescale 1ns/1ps
module tb; logic a; bit ext; int n;
  BODY
  initial begin
    #1;
    forever begin
      wait (ext); $display("%0t woke", $time); ext = 0;
    end
  end
  initial begin a = 0; #5 a = 1; #5 a = 1'bz; #5 a = 0; #5 $finish; end
endmodule
"#;
    for (i, body) in [
        "always @(a) ext = 1;",
        "always @(a) begin ext = 1; end",
        "always @(a) ext = !n;",
    ]
    .iter()
    .enumerate()
    {
        let text = run(&format!("wake{i}"), &src.replace("BODY", body));
        assert_eq!(
            lines_with(&text, "woke"),
            ["1000 woke", "5000 woke", "10000 woke", "15000 woke"],
            "{body}\n{text}"
        );
    }
}

/// Level lists, edge events, an `or` list whose body reads only an unlisted
/// signal, and clocked flops (top-level and in an instance) whose outputs a
/// task and a hierarchical assignment overwrite between edges.
#[test]
fn process_overwrite_does_not_suppress_later_events() {
    let text = run(
        "mixed",
        r#"
`timescale 1ns/1ps
module sub(input logic clk, input logic d);
  logic q;
  always @(posedge clk) q <= d;
endmodule
module tb;
  logic a, b, c, clk, d;
  bit e1, e2, e3, e4, x, q;
  int n;
  sub u(.clk(clk), .d(d));
  always @(a) e1 = 1;
  always @(a) begin e2 = 1; end
  always @(a) e3 = !n;
  always @(posedge a) e4 = 1;
  always @(a or b) x = c;
  always @(posedge clk) q <= d;
  task automatic clear_q; q = 0; endtask
  initial begin
    a = 0; b = 0; c = 1; clk = 0; d = 1; n = 0;
    #1 e1 = 0; e2 = 0; e3 = 0; e4 = 0; x = 0;
    #4 a = 1;
    #1 $display("t=%0t e1=%0d e2=%0d e3=%0d e4=%0d x=%0d", $time, e1, e2, e3, e4, x);
    e1 = 0; e2 = 0; e3 = 0; e4 = 0; x = 0;
    #4 a = 1'bz;
    #1 $display("t=%0t e1=%0d e2=%0d e3=%0d e4=%0d x=%0d", $time, e1, e2, e3, e4, x);
    e1 = 0; e2 = 0; e3 = 0; e4 = 0; x = 0;
    #4 a = 0;
    #1 $display("t=%0t e1=%0d e2=%0d e3=%0d e4=%0d x=%0d", $time, e1, e2, e3, e4, x);
    x = 0;
    #4 b = 1;
    #1 $display("t=%0t x=%0d", $time, x);
    x = 0;
    #4 a = 1;
    #1 $display("t=%0t e4=%0d x=%0d", $time, e4, x);
    #4 clk = 1;
    #1 clk = 0; clear_q(); u.q = 0;
    #1 clk = 1;
    #1 $display("t=%0t q=%0d u.q=%0d", $time, q, u.q);
    clk = 0; q = 0; u.q = 0;
    @(posedge clk);
    #1 $display("t=%0t q=%0d u.q=%0d", $time, q, u.q);
    $finish;
  end
  initial begin #40; forever #5 clk = ~clk; end
endmodule
"#,
    );
    assert_eq!(
        lines_with(&text, "t="),
        [
            "t=6000 e1=1 e2=1 e3=1 e4=1 x=1",
            "t=11000 e1=1 e2=1 e3=1 e4=0 x=1",
            "t=16000 e1=1 e2=1 e3=1 e4=0 x=1",
            "t=21000 x=1",
            "t=26000 e4=1 x=1",
            "t=33000 q=1 u.q=1",
            "t=46000 q=1 u.q=1",
        ],
        "{text}"
    );
}

/// A clocked stimulus loop runs as a compiled process FSM, not on the AST
/// path; its writes count as a driver of the flop's output all the same.
#[test]
fn fsm_stimulus_overwrite_does_not_suppress_flop() {
    let text = run(
        "fsm",
        r#"
`timescale 1ns/1ps
module tb;
  logic clk = 0, d = 1; logic q;
  always @(posedge clk) q <= d;
  always #5 clk = ~clk;
  initial begin
    @(posedge clk); @(negedge clk); q = 0;
    @(posedge clk); #1 $display("t=%0t q=%0d", $time, q);
    @(negedge clk); q = 0;
    @(posedge clk); #1 $display("t=%0t q=%0d", $time, q);
    $finish;
  end
endmodule
"#,
    );
    assert_eq!(
        lines_with(&text, "t="),
        ["t=16000 q=1", "t=26000 q=1"],
        "{text}"
    );
}
