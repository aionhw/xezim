//! IEEE 1800-2023 §9.4.2.3 event-control `iff` guard on an `always` block.
//!
//! `always @(posedge clk iff en) q <= q + 1;` must only advance on the clock
//! edges where `en` is high at edge time. xezim used to parse the guard but
//! fire the edge block unconditionally (the guard was carried on the
//! sensitivity but never evaluated in the edge-detection path).

use xezim::simulate;

const SRC: &str = r#"
`timescale 1ns/1ns
module tb;
  reg clk = 0;
  reg en  = 0;
  reg [31:0] cnt = 0;
  always #5 clk = ~clk;            // posedges at t = 5, 15, 25, 35, ...

  // en is high only during t in (12, 28): covers the posedges at t=15 and
  // t=25 (2 counts) and excludes t=5, t=35, t=45.
  initial begin
    #12 en = 1'b1;
    #16 en = 1'b0;                 // en drops at t=28
  end

  always @(posedge clk iff en) cnt <= cnt + 1;

  initial #60 $finish;
endmodule
"#;

fn lookup(sim: &xezim::compiler::Simulator, name: &str) -> u64 {
    sim.get_signal(name)
        .or_else(|| sim.get_signal(&format!("tb.{}", name)))
        .unwrap_or_else(|| panic!("signal not found: {}", name))
        .to_u64()
        .unwrap_or_else(|| panic!("signal {} not u64-able", name))
}

#[test]
fn always_iff_guard_gates_edge_firing() {
    let sim = simulate(SRC, 200).expect("simulate failed");
    let cnt = lookup(&sim, "cnt") & 0xFFFFFFFF;
    // Only the posedges at t=15 and t=25 occur while en==1 → 2 increments.
    // Without the guard, all posedges through t=55 would count (6+).
    assert_eq!(
        cnt, 2,
        "always @(posedge clk iff en) should only fire while en is high (expected 2), got {}",
        cnt
    );
}
