//! §6.6.7 — user-defined nettype resolution.
//!
//! A nettype's resolver function receives every simultaneous continuous driver
//! as an unpacked queue and returns the resolved value; the elaborator must
//! emit a resolver CALL (not a hardcoded OR-fold of the drivers), and the
//! simulator must scatter the returned value across the net's member leaves.

use xezim::simulate;

const SRC: &str = r#"
module tb;
  typedef logic [7:0] byte_t;

  // LRM §6.6.7 example resolver: sum of the driver queue.
  function automatic byte_t bsum(input byte_t drivers[]);
    byte_t r = 8'h0;
    foreach (drivers[i]) r = r + drivers[i];
    return r;
  endfunction
  nettype byte_t wBSum with bsum;

  wBSum s;   // single driver
  wBSum m;   // two drivers — resolver must sum, not OR-fold

  assign s = 8'h03;
  assign m = 8'h12;
  assign m = 8'h14;   // 0x12 + 0x14 = 0x26, but 0x12 | 0x14 = 0x16

  logic [7:0] s_out = 8'h0;
  logic [7:0] m_out = 8'h0;

  always_comb begin
    s_out = s;
    m_out = m;
  end
endmodule
"#;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able", n))
}

#[test]
fn nettype_resolver_is_called_not_or_folded() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    // Single driver: resolver echoes it.
    assert_eq!(u(&sim, "s_out"), 0x03, "single-driver nettype");
    // Two drivers: the resolver SUMS (0x12 + 0x14 = 0x26). An OR-fold of the
    // drivers would give 0x16, so this distinguishes resolver dispatch from
    // the old hardcoded OR-fold.
    assert_eq!(u(&sim, "m_out"), 0x26, "multi-driver nettype resolver sum");
}
