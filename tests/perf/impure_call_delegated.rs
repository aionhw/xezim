//! §13.4: a timed process that calls a function which writes module
//! variables or calls a system task stays compiled; the call runs the
//! function on the interpreter (`CallFreeFunction`), once per execution, so
//! the function's side effects are exactly the interpreter's (#1328). Before,
//! such a call was `Expr_Call_impure` and the whole process was interpreted.
use std::process::Command;

fn run(src: &str) -> String {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim_impure_call_{}_{}",
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
        .env("XEZIM_FALLBACK_SITES", "1")
        .output()
        .expect("run xezim");
    let _ = std::fs::remove_dir_all(&dir);
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "simulation failed:\n{text}");
    text
}

fn assert_compiled(text: &str) {
    assert!(
        text.lines().any(|l| l.starts_with("[PROC-FSM] registered")),
        "the timed process was not compiled:\n{text}"
    );
    assert!(
        !text.lines().any(|l| l.contains("gated out")),
        "a process was gated out of compilation:\n{text}"
    );
    assert!(
        !text.lines().any(|l| l.contains("Expr_Call_impure")),
        "the call fell back to the interpreter:\n{text}"
    );
}

/// A function that counts its calls in a module variable and warns once
/// through a module flag: the shape of a generated model's helper.
const WRITES_MODULE_STATE: &str = r#"
`timescale 1ns/1ns
module top;
  real x = 0.25;
  int  n = 0;
  int  calls = 0;
  bit  warned = 0;
  function automatic real f(input real a);
    calls = calls + 1;
    if (!warned) begin
      warned = 1;
      $display("W|once|%0d", calls);
    end
    return a * 0.999 + 1.0e-3;
  endfunction
  always #(1) begin
    x = f(x);
    n = n + 1;
    if (n == 100) begin
      $display("C|%0d|%0d|%016h", n, calls, $realtobits(x));
      $finish;
    end
  end
endmodule
"#;

#[test]
fn a_function_writing_module_state_keeps_its_caller_compiled() {
    let text = run(WRITES_MODULE_STATE);
    assert_compiled(&text);
    let once: Vec<&str> = text.lines().filter(|l| l.starts_with("W|")).collect();
    assert_eq!(once, ["W|once|1"], "{text}");
    let mut x = 0.25_f64;
    for _ in 0..100 {
        x = x * 0.999 + 1.0e-3;
    }
    let want = format!("C|100|100|{:016x}", x.to_bits());
    assert!(text.lines().any(|l| l == want), "want {want}:\n{text}");
}

/// A void function called as a statement, with a `$warning` taken on half
/// the calls (§20.10).
const VOID_WITH_WARNING: &str = r#"
`timescale 1ns/1ns
module top;
  real x = 0.0;
  int  n = 0;
  function automatic void check(input real v);
    if (v > 10.0) $warning("high %0d", $rtoi(v));
  endfunction
  always #(1) begin
    x = x + 1.0;
    check(x);
    n = n + 1;
    if (n == 20) $finish;
  end
endmodule
"#;

#[test]
fn a_void_function_with_a_warning_keeps_its_caller_compiled() {
    let text = run(VOID_WITH_WARNING);
    assert_compiled(&text);
    let highs: Vec<String> = text
        .lines()
        .filter_map(|l| l.find("high ").map(|i| l[i..].to_string()))
        .collect();
    let want: Vec<String> = (11..=20).map(|v| format!("high {v}")).collect();
    assert_eq!(highs, want, "{text}");
}

/// Two instances, each with its own copy of a function that writes its own
/// instance's variables: every call must reach its instance's copy.
const TWO_INSTANCES: &str = r#"
`timescale 1ns/100ps
module stage #(parameter real K = 1.0) ();
  real y = 0.0;
  int  hits = 0;
  function automatic real g(input real a);
    hits = hits + 1;
    return a + K;
  endfunction
  always #(1) begin
    y = g(y);
    y = y + 0.0;
  end
endmodule
module top;
  stage #(.K(1.0)) c0 ();
  stage #(.K(2.0)) c1 ();
  initial begin
    #(10.5);
    $display("I|%0d|%0d|%0d|%0d", $rtoi(c0.y), c0.hits, $rtoi(c1.y), c1.hits);
    $finish;
  end
endmodule
"#;

#[test]
fn each_instance_calls_its_own_copy() {
    let text = run(TWO_INSTANCES);
    assert!(text.lines().any(|l| l == "I|10|10|20|10"), "{text}");
}
