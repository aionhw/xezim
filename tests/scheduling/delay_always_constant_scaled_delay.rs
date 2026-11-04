//! §9.4.1 + §22.7: `always #d stmt` with a constant non-literal delay.
//!
//! A single blocking assignment under a delay takes the fast retained-delay
//! path, which compiles the delay expression to bytecode on its own. Under a
//! `timescale whose unit is not the global tick, the elaborator rewrites a
//! bare `#1` to `1 * 10.0` (1ns/100ps); the bytecode peepholes folded that
//! to a constant nothing in the block read, deleted it, and the delay read a
//! stale register — 0 — so the process looped at zero delay (stall warnings,
//! and a counter at 99998 instead of 9). `always begin #1 ... end` and
//! literal delays were unaffected, so it looked timescale-specific.
//!
//! Each check samples between wakeups (`#9.5`, `#19`): at a time the counter
//! also wakes, the order of the two processes is indeterminate (§4.7).

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    simulate(src, 10_000_000)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|l| l.starts_with("R "))
        .collect()
}

#[test]
fn bare_delay_under_a_finer_timescale_advances_time() {
    let out = lines(
        r#"
`timescale 1ns/100ps
module top;
  int a = 0;
  always #1 a = a + 1;
  initial begin #9.5 $display("R %0d", a); $finish; end
endmodule
"#,
    );
    assert_eq!(out, vec!["R 9"]);
}

#[test]
fn constant_expression_delay_advances_time() {
    let out = lines(
        r#"
module top;
  int a = 0, b = 0;
  always #(1*2.0) a = a + 1;
  always #(1*2) b = b + 1;
  initial begin #19 $display("R %0d %0d", a, b); $finish; end
endmodule
"#,
    );
    assert_eq!(out, vec!["R 9 9"]);
}
