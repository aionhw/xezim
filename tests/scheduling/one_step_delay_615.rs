//! IEEE 1800-2023 A.2.2.3 / §3.14.3: `1step` is a delay value, one tick of
//! the global time precision. It lexed as `1` and `step`, so `a = #1step b;`
//! was a parse error and `#1step;` waited one time unit (#615, reported by
//! git-hevangel). Every delay form now takes it, unrounded by a coarser
//! scope's precision; `x = 1step;` is still an error.
use xezim::simulate;

fn shown(src: &str, f: impl Fn(&str) -> bool) -> Vec<String> {
    simulate(src, 1000)
        .expect("simulate")
        .output
        .iter()
        .map(|l| l.message.clone())
        .filter(|m| f(m))
        .collect()
}

#[test]
fn every_delay_form_takes_one_step() {
    let out = shown(
        r#"// IEEE 1800-2023 A.2.2.3: delay_value ::= ... | 1step. A step is the global
// time precision (3.14.3): the smallest precision of any timescale in the
// design. Every timescale below has the precision 1 ps, so a step is 1 ps in
// every scope, whatever the time unit of the scope that writes it (1 ns here,
// 100 ns in `wide`).
//
// Every line should print PASS. With the bug the file does not compile:
// `a = #1step b;` stops at the identifier `step`.
`timescale 100ns/1ps
module wide;
  logic [7:0] r;
  wire        w;
  reg         src = 0;
  realtime    t0, d_intra, d_ctl, d_assign;   // in units of 100 ns
  assign #1step w = src;
  task automatic run;
    t0 = $realtime;
    r = #1step 8'h42;
    d_intra = $realtime - t0;
    t0 = $realtime;
    #1step;
    d_ctl = $realtime - t0;
    t0 = $realtime;
    src = 1;
    @(w);
    d_assign = $realtime - t0;
  endtask
endmodule

`timescale 1ns/1ps
module t;
  wide u();
  logic [7:0] a, b, c;
  wire        y, g;
  reg         x = 0;
  realtime    t0, d;
  int         fails = 0;

  assign #1step y = x;
  buf #1step gate1 (g, x);

  task automatic check(string name, bit ok, string info = "");
    if (ok) $display("PASS %s", name);
    else begin $display("FAIL %s %s", name, info); fails++; end
  endtask

  // within 1 % of the wanted delay
  function automatic bit near(realtime got, realtime want);
    return got > want * 0.99 && got < want * 1.01;
  endfunction

  initial begin
    b = 8'hc3; a = 0; c = 0;
    #10;

    t0 = $realtime;  a = #1step b;  d = $realtime - t0;
    check("intra_blocking_value", a === 8'hc3);
    check("intra_blocking_delay", near(d, 0.001), $sformatf("delta=%f ns", d));

    t0 = $realtime;  c <= #1step b;  @(c);  d = $realtime - t0;
    check("intra_nonblocking_delay", near(d, 0.001), $sformatf("delta=%f ns", d));

    t0 = $realtime;  #1step;  d = $realtime - t0;
    check("delay_control_delay", near(d, 0.001), $sformatf("delta=%f ns", d));

    t0 = $realtime;  #1step c = 8'h07;  d = $realtime - t0;
    check("delay_control_then_statement", near(d, 0.001) && c === 8'h07,
          $sformatf("delta=%f ns c=%h", d, c));

    t0 = $realtime;  #(1step) c = 8'h08;  d = $realtime - t0;
    check("delay_control_parenthesized", near(d, 0.001) && c === 8'h08,
          $sformatf("delta=%f ns c=%h", d, c));

    t0 = $realtime;  x = 1;  @(y);  d = $realtime - t0;
    check("continuous_assign_delay", near(d, 0.001), $sformatf("delta=%f ns", d));
    t0 = $realtime;  x = 0;  @(g);  d = $realtime - t0;
    check("gate_delay", near(d, 0.001), $sformatf("delta=%f ns", d));

    // the 100 ns scope: the same 1 ps is 0.00001 of its unit
    u.run;
    check("wide_intra_value", u.r === 8'h42);
    check("wide_intra_delay", near(u.d_intra, 0.00001), $sformatf("delta=%g units", u.d_intra));
    check("wide_delay_control", near(u.d_ctl, 0.00001), $sformatf("delta=%g units", u.d_ctl));
    check("wide_continuous_assign", near(u.d_assign, 0.00001), $sformatf("delta=%g units", u.d_assign));

    if (fails == 0) $display("ALL PASS"); else $display("%0d FAILED", fails);
    $finish;
  end
endmodule
"#,
        |m| m.contains("PASS") || m.contains("FAIL"),
    );
    assert!(out.iter().any(|m| m == "ALL PASS"), "{out:?}");
    assert!(!out.iter().any(|m| m.contains("FAIL")), "{out:?}");
}

/// A variable named `step` stays untouched, and a 1 ns precision scope
/// still waits one global step (1 ps).
#[test]
fn a_step_is_the_global_precision() {
    assert_eq!(
        shown(
            r#"`timescale 1ns/1ps
module coarse;
  timeunit 1ns; timeprecision 1ns;
  real t0, d; logic a, b;
  initial begin #5; t0 = $realtime; a = #1step b; d = $realtime - t0; $display("C %f", d); end
endmodule
module t;
  int step = 3;
  real t0, d;
  coarse u();
  initial begin
    t0 = $realtime; #1step; d = $realtime - t0;
    $display("S %f %0d", d, step);
  end
endmodule
"#,
            |m| m.starts_with("S ") || m.starts_with("C ")
        ),
        ["S 0.001000 3", "C 0.001000"]
    );
}

#[test]
fn one_step_is_not_a_value() {
    assert!(
        simulate(
            r#"module t; int x; initial x = 1step; endmodule
"#,
            10
        )
        .is_err()
    );
}

/// §3.14.3: in a 1ns-precision module of a design whose global precision is
/// 1ps, `#1step` waits 1ps, through the compiled process path too.
#[test]
fn one_step_in_a_coarse_scope_is_one_global_tick() {
    let out: Vec<String> = simulate(
        r#"`timescale 1ns/1ns
module m;
  int n = 0;
  initial begin
    #1step n = 1;
    #1step n = 2;
    #1 n = 3;
  end
endmodule
`timescale 1ns/1ps
module t;
  m u();
  always @(u.n) $display("R %0d at %0.3f", u.n, $realtime);
endmodule
"#,
        10_000,
    )
    .expect("simulate")
    .output
    .iter()
    .map(|l| l.message.clone())
    .filter(|m| m.starts_with("R "))
    .collect();
    assert_eq!(out, ["R 1 at 0.001", "R 2 at 0.002", "R 3 at 1.002"]);
}
