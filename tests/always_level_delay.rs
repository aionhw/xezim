//! A LEVEL-sensitive always with a delayed (blocking) body — `always @(a) #5
//! z=a` — must schedule like an edge-triggered block, NOT the comb-settle path.
//! The comb path strips the `@()` and re-runs the delayed body in a zero-time
//! settle loop, which used to spin and STARVE every initial block (nothing ran)
//! and never propagate the assignment. Now the delayed body suspends correctly
//! and the value propagates, while initials run normally.

use xezim::simulate;

fn out(src: &str) -> String {
    let sim = simulate(src, 100000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect::<Vec<_>>().join("\n")
}

/// The delayed level-sensitive always propagates its assignment.
#[test]
fn delayed_level_always_propagates() {
    let o = out(r#"
module t;
  reg a=0, z=0;
  always @(a) #5 z=a;
  initial begin #1 a=1; #20 $display("Z=%0b", z); $finish; end
endmodule
"#);
    assert!(o.contains("Z=1"), "delayed level-always must propagate a->z; got: {}", o);
}

/// Initial blocks must NOT be starved by the presence of such an always.
#[test]
fn initials_not_starved_by_delayed_level_always() {
    let o = out(r#"
module t;
  reg a=0, z=0;
  always @(a) #5 z=a;
  initial $display("INIT0");
  initial #1 $display("INIT1");
endmodule
"#);
    assert!(o.contains("INIT0"), "t=0 initial must run; got: {}", o);
    assert!(o.contains("INIT1"), "delayed initial must run; got: {}", o);
}

/// Cross-layer, multi-timescale hierarchy: a leaf `always @(a) #5 z=a` (1ps)
/// driven from a 1us top must deliver the transition to the top. This is the
/// exact multi-layer-DUT flow; timing matches a reference simulator.
#[test]
fn cross_layer_multitimescale_flow() {
    let o = out(r#"
`timescale 1us/1ns
module top;
  reg a=0; wire z;
  mid m(.a(a), .z(z));
  initial begin #1 a=1; end
  initial begin @(posedge z) $display("SEEN=%0g", $realtime); $finish; end
  initial begin #100 $display("TIMEOUT"); $finish; end
endmodule
`timescale 1ns/1ps
module mid(input a, output z); leaf l(.a(a), .z(z)); endmodule
`timescale 1ps/1ps
module leaf(input a, output reg z); initial z=0; always @(a) #5 z=a; endmodule
"#);
    assert!(o.contains("SEEN="), "top must observe z's posedge (not TIMEOUT); got: {}", o);
    assert!(!o.contains("TIMEOUT"), "must not time out; got: {}", o);
}

/// Regression: a plain (non-delayed) level always still uses the comb path.
#[test]
fn plain_level_always_still_works() {
    let o = out(r#"
module t;
  reg a=0, z=0;
  always @(a) z=a;
  initial begin @(posedge z) $display("PZ=%0t", $time); $finish; end
  initial begin #1 a=1; end
endmodule
"#);
    assert!(o.contains("PZ=1"), "no-delay level-always must still fire at t=1; got: {}", o);
}
