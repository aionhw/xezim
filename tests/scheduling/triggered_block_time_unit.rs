//! §20.3.1: `$time` (and `$stime`/`$realtime`) returns the time "scaled to
//! the time unit of the module that invoked it" — the module the CALL is
//! written in, whatever process happens to evaluate it.
//!
//! A blocking write settles its fanout eagerly, inside the writing process:
//! a level-sensitive `always @(b)`, an `always_comb`, a continuous assign and
//! an edge block woken by the write all run before the writer resumes. Each
//! of those installed its own scope as the timescale owner only when it had
//! a non-empty scope hint, so a TOP-level block (no hint) fell through to the
//! running process — the writer — and counted the WRITER's timeunit: a
//! `1ns` testbench reacting to a write from a `1fs` submodule printed
//! `$time` = 5000000 instead of 5. Likewise a submodule's continuous assign
//! (elaborated with fully qualified names, so no hint either) counted the
//! testbench's unit, and a top-level function called from a submodule as
//! `tb.now()` counted the submodule's.
//!
//! Expected values were measured on the reference simulator. Lines are
//! compared sorted: the order of same-time-step processes is not specified
//! (§4.7).

use xezim::simulate;

/// The `NOTE:` lines a run printed, sorted.
fn notes(src: &str) -> Vec<String> {
    let sim = simulate(src, 100_000_000).expect("simulate failed");
    let mut v: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|l| l.starts_with("NOTE:"))
        .collect();
    v.sort();
    v
}

/// The original report: a `1ns` testbench's `always @(b)` woken by a write
/// from a `1fs` submodule.
#[test]
fn top_always_woken_by_finer_submodule_counts_top_unit() {
    let src = r#"
`timescale 1ns/1ns
module tb;
  logic b;
  sub u(b);
  initial begin #10; $display("NOTE: tb time=%0d", $time); end
  always @(b) $display("NOTE: tb always b=%b time=%0d", b, $time);
endmodule
module sub(output logic b);
  timeunit 1fs; timeprecision 1fs;
  initial begin #5000000 b = 1; $display("NOTE: sub time=%0d", $time); end
endmodule
"#;
    assert_eq!(
        notes(src),
        vec![
            "NOTE: sub time=5000000",
            "NOTE: tb always b=1 time=5",
            "NOTE: tb time=10",
        ]
    );
}

/// Every construct the write can wake in the testbench: a continuous assign
/// calling a function that reads `$time`, `always_comb` with `$time` and
/// `$realtime`, `always @(b)` with all three system functions, an edge block
/// clocked by the submodule's output port — and a top-level function the
/// submodule calls hierarchically.
#[test]
fn top_constructs_woken_by_finer_submodule_count_top_unit() {
    let src = r#"
`timescale 1ns/1ps
module tb;
  logic b, clk;
  logic [63:0] ca, cc;
  real rt;
  function automatic logic [63:0] now(input logic x); return $time; endfunction
  assign ca = now(b);
  always_comb cc = b ? $time : 0;
  always_comb rt = b ? $realtime : 0.0;
  sub u(.b(b), .clk(clk));
  always @(b) if (b) $display("NOTE: always time=%0d stime=%0d rt=%0.3f", $time, $stime, $realtime);
  always @(posedge clk) $display("NOTE: posedge time=%0d", $time);
  initial #20 $display("NOTE: end ca=%0d cc=%0d rt=%0.3f", ca, cc, rt);
endmodule
module sub(output logic b, output logic clk);
  timeunit 1ps; timeprecision 1ps;
  initial begin
    b = 0; clk = 0;
    #5500 b = 1;
    #1000 clk = 1;
    #1000 $display("NOTE: tb.now from sub=%0d", tb.now(1'b0));
  end
endmodule
"#;
    assert_eq!(
        notes(src),
        vec![
            "NOTE: always time=6 stime=6 rt=5.500",
            "NOTE: end ca=6 cc=6 rt=5.500",
            "NOTE: posedge time=7",
            "NOTE: tb.now from sub=8",
        ]
    );
}

/// The reverse direction: a `1ps` submodule's assign, function-calling
/// assign, `always_comb` and blocks woken by a write from the `1ns`
/// testbench count the SUBMODULE's unit.
#[test]
fn submodule_constructs_woken_by_coarser_top_count_their_own_unit() {
    let src = r#"
`timescale 1ns/1ps
module tb;
  logic a;
  sub u(.a(a));
  initial begin
    a = 0;
    #5.5 a = 1;
    #10 $display("NOTE: end t=%0d g=%0d c=%0d", u.t, u.g, u.c);
  end
endmodule
module sub(input logic a);
  timeunit 1ps; timeprecision 1ps;
  logic [63:0] t, c, g;
  function automatic logic [63:0] f(input logic x); return x ? $time : 0; endfunction
  assign t = a ? $time : 0;
  assign g = f(a);
  always_comb c = a ? $time : 0;
  always @(a) if (a) $display("NOTE: always time=%0d", $time);
  always @(posedge a) $display("NOTE: posedge time=%0d", $time);
endmodule
"#;
    assert_eq!(
        notes(src),
        vec![
            "NOTE: always time=5500",
            "NOTE: end t=5500 g=5500 c=5500",
            "NOTE: posedge time=5500",
        ]
    );
}

/// A submodule function called from a testbench block woken by the
/// submodule's own write counts the submodule's unit (§20.3.1: the module
/// the function is declared in), while the block's own `$time` counts the
/// testbench's.
#[test]
fn hierarchical_function_counts_declaring_module_unit() {
    let src = r#"
`timescale 1ns/1ps
module tb;
  logic b;
  sub u(.b(b));
  always @(b) if (b) $display("NOTE: always time=%0d f=%0d", $time, u.subnow());
  initial #20 $display("NOTE: end f=%0d", u.subnow());
endmodule
module sub(output logic b);
  timeunit 1ps; timeprecision 1ps;
  function automatic logic [63:0] subnow(); return $time; endfunction
  initial begin b = 0; #5500 b = 1; end
endmodule
"#;
    assert_eq!(
        notes(src),
        vec!["NOTE: always time=6 f=5500", "NOTE: end f=20000"]
    );
}
