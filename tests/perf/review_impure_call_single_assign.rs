//! §13.4 (#1328): the single-assignment form `always #(d) x = f(x);` keeps
//! its retained body compiled when `f` writes module variables or runs a
//! system task. The call runs the function on the interpreter once per
//! wakeup, so every side effect, and every process woken by one, is the
//! interpreter's.
use std::process::Command;

fn run(src: &str) -> String {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim_impure_single_{}_{}",
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
            "--sim-debug",
        ])
        .output()
        .expect("run xezim");
    let _ = std::fs::remove_dir_all(&dir);
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "simulation failed:\n{text}");
    text
}

fn lines(text: &str) -> Vec<&str> {
    text.lines().filter(|l| l.starts_with("T|")).collect()
}

const SIDE_EFFECTS: &str = r#"
`timescale 1ns/100ps
module top;
  real x = 0.25;
  int n = 0, calls = 0;
  bit warned = 0;
  function automatic real f(input real a);
    calls = calls + 1;
    if (!warned) begin
      warned = 1;
      $display("T|once %m %0d", calls);
    end
    return a * 0.5 + 1.0;
  endfunction
  always #(1) x = f(x);
  always @(calls) n = n + 1;
  initial begin
    #(3.5);
    $display("T|%0d %0d %016h", calls, n, $realtobits(x));
    $finish;
  end
endmodule
"#;

#[test]
fn a_retained_assignment_calling_a_function_with_side_effects_compiles() {
    let text = run(SIDE_EFFECTS);
    assert!(
        text.contains("[OPT] bytecode compiled: 1/1 retained dynamic-delay bodies"),
        "the retained body was not compiled:\n{text}"
    );
    let mut x = 0.25_f64;
    for _ in 0..3 {
        x = x * 0.5 + 1.0;
    }
    let want = format!("T|3 3 {:016x}", x.to_bits());
    assert_eq!(lines(&text), ["T|once top.f 1", want.as_str()]);
}
