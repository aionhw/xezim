//! A function body is its own scope (§13.4): the names it reads are its own
//! formals and locals, or the names of the scope that declares it, never the
//! caller's. The compiled path unrolls a loop whose body calls a function by
//! binding the loop variable as a constant per trip, and then inlines the
//! call. The constant stayed visible inside the inlined body, where it shadows
//! every other name. So a function whose own `for (int s ...)` loop shared the
//! caller's loop variable name tested the caller's constant: `s < N` never
//! became false and the run hung at time zero, or the loop never ran. And a
//! function reading a module variable `s` read the caller's `s` instead.
//! Inlined tasks already hid the caller's constants.
//!
//! Each case runs as a subprocess with a deadline, so a hang fails the test
//! instead of stalling the suite. Values are from the reference simulator.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Run `src` with top `top`; the `T|` lines it prints, or `None` when the
/// run had not finished after 60 s.
fn run(src: &str, top: &str) -> Option<Vec<String>> {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim_inlconst_{}_{}",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("tb.sv");
    std::fs::write(&path, src).unwrap();
    let out_path = dir.join("out.txt");
    let mut child = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["-s", top, path.to_str().unwrap(), "--no-cache"])
        .stdout(Stdio::from(std::fs::File::create(&out_path).unwrap()))
        .stderr(Stdio::null())
        .spawn()
        .expect("run xezim");
    let start = Instant::now();
    let finished = loop {
        if child.try_wait().expect("wait").is_some() {
            break true;
        }
        if start.elapsed() > Duration::from_secs(60) {
            let _ = child.kill();
            let _ = child.wait();
            break false;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let text = std::fs::read_to_string(&out_path).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&dir);
    finished.then(|| {
        text.lines()
            .filter(|l| l.starts_with("T|"))
            .map(str::to_string)
            .collect()
    })
}

fn expect(src: &str, top: &str, want: &[&str]) {
    let got = run(src, top).expect("the run did not finish within 60 s");
    assert_eq!(got, want);
}

#[test]
fn a_function_loop_named_like_the_callers_loop_variable_returns() {
    // The evaluation report's case: an instanced module, a parameter bound,
    // both loop variables `int unsigned s`. It hung at time zero.
    let src = r#"
module m #(parameter int unsigned N = 2);
  int unsigned r;
  function automatic int unsigned f(input int unsigned x);
    f = 0;
    for (int unsigned s = 0; s < N; s++) f = f + 1;
  endfunction
  initial begin
    for (int unsigned s = 0; s < 3; s++) r = f(s);
    $display("T|r=%0d", r);
  end
endmodule
module top; m u (); endmodule
"#;
    expect(src, "top", &["T|r=2"]);
}

#[test]
fn a_function_loop_does_not_test_the_callers_loop_constant() {
    // The caller's `s` starts above the bound, so the leaked constant ended
    // the callee's loop before its first trip: 0 per call instead of 2.
    let src = r#"
module m #(parameter int N = 2);
  int acc;
  function automatic int count(input int x);
    count = 0;
    for (int s = 0; s < N; s++) count = count + 1;
  endfunction
  initial begin
    for (int s = 5; s < 8; s++) acc += count(s);
    $display("T|acc=%0d", acc);
  end
endmodule
module top; m u (); endmodule
"#;
    expect(src, "top", &["T|acc=6"]);
}

#[test]
fn a_function_reads_the_module_variable_not_the_callers_loop_constant() {
    // §13.4: `s` in the body names the module variable (40), not the
    // caller's loop variable.
    let src = r#"
module top;
  int s = 40;
  int acc;
  function automatic int modread(input int x);
    return s + x;
  endfunction
  initial begin
    for (int s = 0; s < 3; s++) acc += modread(s);
    $display("T|acc=%0d", acc);
  end
endmodule
"#;
    expect(src, "top", &["T|acc=123"]);
}

#[test]
fn locals_formals_tasks_and_other_names_keep_working() {
    // Guard: a local and a formal named like the caller's loop variable, an
    // inlined task, and a loop variable of another name.
    let src = r#"
module m;
  int a_local, a_formal, a_task, a_k;
  function automatic int local_s(input int x);
    int s = x * 3;
    return s + 1;
  endfunction
  function automatic int formal_s(input int s);
    return s * 2;
  endfunction
  task automatic task_s(input int x, output int y);
    int s = x * 3;
    y = s + 1;
  endtask
  initial begin
    int y;
    for (int s = 0; s < 3; s++) a_local += local_s(s);
    for (int s = 0; s < 3; s++) a_formal += formal_s(s + 1);
    for (int s = 0; s < 3; s++) begin task_s(s, y); a_task += y; end
    for (int k = 0; k < 3; k++) a_k += local_s(k);
    $display("T|local=%0d formal=%0d task=%0d k=%0d", a_local, a_formal, a_task, a_k);
  end
endmodule
module top; m u (); endmodule
"#;
    expect(src, "top", &["T|local=12 formal=12 task=12 k=12"]);
}
