//! §9.2.2.1, §9.4.1: a free-running delay loop whose delay is not at its
//! head, `always begin #(d); ... end` or `initial forever begin ...; #(d);
//! end`, is the same process as `always #(d) ...` and becomes a process FSM
//! by default like it (#1327). A delay sequence that ends (the stop block
//! here) keeps the AST path. Each value is checked bit for bit, printed as
//! `$realtobits` hex.
use std::process::Command;

fn run(src: &str) -> String {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim_delay_loop_{}_{}",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temporary directory");
    let path = dir.join("design.sv");
    std::fs::write(&path, src).expect("write design");
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args([
            "--simulate",
            "-s",
            "top",
            path.to_str().unwrap(),
            "--no-cache",
        ])
        .env("XEZIM_PROC_LOOP_STATS", "1")
        .env_remove("XEZIM_PROC_FSM")
        .output()
        .expect("run xezim");
    let _ = std::fs::remove_dir_all(&dir);
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "simulation failed:\n{text}");
    text
}

/// Exactly one process compiled: the loop, and not the stop block.
fn assert_one_fsm(text: &str) {
    let n = text
        .lines()
        .filter(|l| l.starts_with("[PROC-FSM] registered"))
        .count();
    assert_eq!(n, 1, "want the loop alone compiled:\n{text}");
}

/// A variable head delay, re-evaluated each pass: `d` doubles after the third
/// pass, so the wakeups are 10, 20, 30, 50, 70 and 90.
const VARIABLE_HEAD: &str = r#"
`timescale 1ns/1ns
module top;
  real x = 0.0;
  int  n = 0;
  real d = 10.0;
  always begin
    #(d);
    x = x * 0.5 + 1.0;
    n = n + 1;
    if (n == 3) d = 20.0;
  end
  initial begin
    #(105);
    $display("V|%0d|%016h", n, $realtobits(x));
    $finish;
  end
endmodule
"#;

#[test]
fn always_with_a_variable_delay_first_compiles() {
    let text = run(VARIABLE_HEAD);
    assert_one_fsm(&text);
    let mut x = 0.0_f64;
    for _ in 0..6 {
        x = x * 0.5 + 1.0;
    }
    let want = format!("V|6|{:016x}", x.to_bits());
    assert!(text.lines().any(|l| l == want), "want {want}:\n{text}");
}

/// Untimed set-up, then a `forever` with two delays: `x` scales at 0, 10,
/// ..., 100 (11 times) and `n` counts at 5, 15, ..., 95 (10 times).
const FOREVER_TWO_DELAYS: &str = r#"
`timescale 1ns/1ns
module top;
  real x;
  int  n = 0;
  real d = 10.0;
  initial begin
    x = 1.0;
    forever begin
      x = x * 0.75;
      #(d * 0.5);
      n = n + 1;
      #(d * 0.5);
    end
  end
  initial begin
    #(102);
    $display("F|%0d|%016h", n, $realtobits(x));
    $finish;
  end
endmodule
"#;

#[test]
fn initial_forever_with_inner_delays_compiles() {
    let text = run(FOREVER_TWO_DELAYS);
    assert_one_fsm(&text);
    let mut x = 1.0_f64;
    for _ in 0..11 {
        x *= 0.75;
    }
    let want = format!("F|10|{:016x}", x.to_bits());
    assert!(text.lines().any(|l| l == want), "want {want}:\n{text}");
}

#[test]
fn delayed_nba_captures_value_and_subscript_in_a_compiled_loop() {
    let text = run(r#"
`timescale 1ns/1ns
module top;
  int n = 0;
  int src = 10;
  int idx = 0;
  int dst[2];
  always #10 begin
    n += 1;
    if (n == 1) begin
      dst[idx] <= #3 src;
      idx = 1;
      src = 99;
    end
  end
  initial begin
    #12 $display("N|early %0d %0d %0d", n, dst[0], dst[1]);
    #2 $display("N|late %0d %0d %0d", n, dst[0], dst[1]);
    $finish;
  end
endmodule
"#);
    assert_one_fsm(&text);
    assert!(text.lines().any(|l| l == "N|early 1 0 0"), "{text}");
    assert!(text.lines().any(|l| l == "N|late 1 10 0"), "{text}");
}

/// A labeled block is a hierarchical scope (`disable named`, `named.v`) the
/// FSM does not model, so a labeled `initial forever` keeps the AST path.
const LABELED: &str = r#"
`timescale 1ns/1ns
module top;
  int m = 0;
  initial begin : named
    forever begin
      #(3);
      m = m + 1;
    end
  end
  initial begin
    #(101);
    $display("L|%0d", m);
    $finish;
  end
endmodule
"#;

#[test]
fn a_labeled_initial_forever_keeps_the_ast_path() {
    let text = run(LABELED);
    assert!(
        !text.lines().any(|l| l.starts_with("[PROC-FSM] registered")),
        "a labeled block was compiled:
{text}"
    );
    assert!(text.lines().any(|l| l == "L|33"), "{text}");
}
