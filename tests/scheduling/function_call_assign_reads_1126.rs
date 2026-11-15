//! §10.3.2, §13.4 (#1126): a continuous assignment that calls a function is
//! evaluated when an operand changes. The called function's own name, its
//! loop variables and its block locals are not operands; counted as
//! unresolved reads they made the assignment evaluate at every settle, so a
//! function of `$time` or with static state changed value on unrelated
//! activity. A function's formal, local or loop variable named like a module
//! parameter is the frame's, not the parameter, when it indexes an array.

use std::path::PathBuf;
use std::process::Command;

fn check(source: &str, expected: &[&str]) {
    let actual: Vec<_> = xezim::simulate(source, 1_000_000)
        .expect("simulate")
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|s| s.starts_with("T|"))
        .collect();
    assert_eq!(actual, expected);
}

/// `entry_evals=` and the unresolved entry count of the `--profile` report.
fn profile(name: &str, source: &str) -> (String, u64, u64) {
    let dir = std::env::temp_dir().join(format!(
        "xezim_function_call_assign_reads_1126_{}_{}",
        name,
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create temp directory");
    let path = dir.join(format!("{name}.sv"));
    std::fs::write(&path, source).expect("write test source");
    let out = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_xezim")))
        .arg("--no-cache")
        .arg("--profile")
        .arg(&path)
        .current_dir(&dir)
        .output()
        .expect("run xezim");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(out.status.success(), "xezim failed: {out:?}");
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let field = |key: &str| -> u64 {
        let at = all
            .find(key)
            .unwrap_or_else(|| panic!("no {key:?} in:\n{all}"))
            + key.len();
        all[at..]
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>()
            .parse()
            .unwrap_or_else(|_| panic!("bad {key:?} in:\n{all}"))
    };
    let evals = field("entry_evals=");
    let unresolved = field("unresolved_entries=");
    let values = all
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("T|"))
        .collect::<Vec<_>>()
        .join("\n");
    (values, evals, unresolved)
}

#[test]
fn function_call_assigns_evaluate_on_operand_change_only() {
    let (values, evals, unresolved) = profile(
        "settle",
        r#"
`timescale 1ns/1ps
module t;
  logic [31:0] frozen = 32'h1234;
  logic [31:0] store [64];
  function automatic logic [31:0] identity(input logic [31:0] v);
    return v;
  endfunction
  function automatic logic [31:0] next_value(input logic [31:0] s [64]);
    logic [31:0] acc = 0;
    foreach (s[i]) acc += s[i];
    for (int j = 0; j < 2; j++) begin
      logic [31:0] one;
      one = 1;
      acc += one;
    end
    return acc;
  endfunction
  wire [31:0] still = identity(frozen);
  wire [31:0] cand = next_value(store);
  logic clk = 0, q = 0;
  always #5 clk = ~clk;
  always @(posedge clk) q <= ~q;
  initial begin
    foreach (store[i]) store[i] = i;
    #5000;
    $display("T|still=%h cand=%0d", still, cand);
    store[3] = 100;
    #1 $display("T|cand=%0d", cand);
    $finish;
  end
endmodule
"#,
    );
    assert_eq!(values, "T|still=00001234 cand=2018\nT|cand=2115");
    assert_eq!(unresolved, 0, "no entry should be unresolved");
    // 500 clock edges: each assignment is evaluated a handful of times,
    // not at every settle.
    assert!(evals < 200, "entry_evals={evals}");
}

#[test]
fn instance_function_call_assigns_evaluate_on_operand_change_only() {
    let (values, _evals, unresolved) = profile(
        "instance",
        r#"
`timescale 1ns/1ps
module unit #(parameter int K = 1) (input logic [31:0] seed, output logic [31:0] cand, output logic [31:0] when);
  logic [31:0] store [8];
  function automatic logic [31:0] next_value(input logic [31:0] s [8]);
    logic [31:0] acc = 0;
    foreach (s[i]) acc += s[i];
    for (int j = 0; j < K; j++) acc += 1;
    return acc;
  endfunction
  function automatic logic [31:0] stamp(input logic [31:0] v);
    return v + int'($time);
  endfunction
  assign cand = next_value(store);
  assign when = stamp(seed);
  initial foreach (store[i]) store[i] = i * K;
endmodule
module t;
  logic [31:0] seed = 5;
  logic [31:0] c1, c2, w1, w2;
  unit #(1) u1 (.seed(seed), .cand(c1), .when(w1));
  unit #(2) u2 (.seed(seed), .cand(c2), .when(w2));
  logic clk = 0, q = 0;
  always #5 clk = ~clk;
  always @(posedge clk) q <= ~q;
  initial begin
    #20 seed = 6;
    #20 u1.store[3] = 100;
    #20;
    $display("T|c1=%0d c2=%0d w1=%0d w2=%0d", c1, c2, w1, w2);
    $finish;
  end
endmodule
"#,
    );
    // `when` takes the time of the last change of `seed` (20).
    assert_eq!(values, "T|c1=126 c2=58 w1=26 w2=26");
    assert_eq!(unresolved, 0, "no entry should be unresolved");
}

#[test]
fn stateful_function_assigns_evaluate_on_operand_change_only() {
    check(
        r#"
`timescale 1ns/1ns
module top;
  logic [31:0] x = 0;
  function automatic int f_time(input logic [31:0] v);
    return int'($time) + v;
  endfunction
  function int f_static(input logic [31:0] v);
    int count;
    count = count + 1;
    return count;
  endfunction
  wire [31:0] y_time = f_time(x);
  wire [31:0] y_static = f_static(x);
  logic clk = 0, q = 0;
  always #5 clk = ~clk;
  always @(posedge clk) q <= ~q;
  initial begin
    #1 x = 1;
    #100;
    $display("T|y_time=%0d y_static=%0d", y_time, y_static);
    $finish;
  end
endmodule
"#,
        &["T|y_time=2 y_static=2"],
    );
}

#[test]
fn always_comb_function_frame_names_shadow_parameters() {
    check(
        r#"
`timescale 1ns/1ns
module top;
  parameter int idx = 1;
  logic [7:0] mem [4];
  logic [7:0] sel = 2;
  function automatic logic [7:0] pick_mod(input int idx);
    return mem[idx];
  endfunction
  function automatic logic [7:0] sum_mod(input logic [7:0] dummy);
    logic [7:0] acc = 0;
    for (int idx = 0; idx < 4; idx++) acc += mem[idx];
    return acc;
  endfunction
  function automatic logic [7:0] sum_each(input logic [7:0] dummy);
    logic [7:0] acc = 0;
    foreach (mem[idx]) acc += mem[idx];
    return acc;
  endfunction
  logic [7:0] c_pick, c_sum, c_each;
  always_comb c_pick = pick_mod(sel);
  always_comb c_sum = sum_mod(8'd0);
  always_comb c_each = sum_each(8'd0);
  initial begin
    mem[0] = 1; mem[1] = 2; mem[2] = 4; mem[3] = 8;
    #1 $display("T|t1 pick=%0d sum=%0d each=%0d", c_pick, c_sum, c_each);
    mem[2] = 40;
    #1 $display("T|t2 pick=%0d sum=%0d each=%0d", c_pick, c_sum, c_each);
    mem[3] = 80;
    #1 $display("T|t3 pick=%0d sum=%0d each=%0d", c_pick, c_sum, c_each);
    $finish;
  end
endmodule
"#,
        &[
            "T|t1 pick=4 sum=15 each=15",
            "T|t2 pick=40 sum=51 each=51",
            "T|t3 pick=40 sum=123 each=123",
        ],
    );
}
