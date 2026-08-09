//! §7.4.1 — nested packed element selects, and instance-name collisions on
//! the packed-metadata tables. Reference-validated (customer-derived MWE,
//! identifiers anonymized).
//!
//! Two stacked defects, found from one field report:
//!  * `x[i][j]` on `T [0:0][1:0] x;` (packed array of packed structs) had
//!    no multi-level select path — the second index degraded to a BIT
//!    select of the first slice, so `$bits(x[0][0])` read 1 and every
//!    nested element read returned one bit.
//!  * an INSTANCE-internal declaration sharing the bare name (a 128-bit
//!    `logic [1:0][63:0] x` inside a submodule) STOMPED the outer signal's
//!    entries in the global bare-name packed-metadata tables at
//!    instantiation-merge time — the testbench then sliced its 20-bit
//!    struct array with the submodule's 64-bit element geometry (values,
//!    not just widths). Bare-key registration is now first-wins; the
//!    instance-scoped key stays authoritative.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} is x/z", n))
}

const SRC: &str = r#"
package P;
  typedef struct packed { logic [1:0][3:0] d; logic [1:0] m; } st_t; // 10 bits
endpackage
module sub(input logic clk);
  logic [1:0][63:0] x;   // colliding bare name, different packed dims
  always_ff @(posedge clk) x <= 128'h5;
endmodule
module tb;
  import P::*;
  P::st_t [0:0][1:0] x;
  logic clk = 0;
  sub u_s(.clk(clk));
  int b_all, b_lvl1, b_lvl2;
  logic [9:0] elem;
  logic [3:0] deep;
  initial begin
    x = 20'hBEEF5;
    b_all  = $bits(x);
    b_lvl1 = $bits(x[0]);
    b_lvl2 = $bits(x[0][0]);
    elem   = x[0][0];
    deep   = x[0][1].d[1];
  end
endmodule
"#;

#[test]
fn packed_multilevel_selects_survive_instance_collision() {
    let sim = simulate(SRC, 50).expect("simulate failed");
    assert_eq!(u(&sim, "b_all"), 20, "$bits(x) whole signal");
    assert_eq!(u(&sim, "b_lvl1"), 20, "$bits(x[0]) — outer dim [0:0] element");
    assert_eq!(u(&sim, "b_lvl2"), 10, "$bits(x[0][0]) — struct element");
    // 20'hBEEF5: element [0][0] is the LOW struct slot = 10'h2F5.
    assert_eq!(u(&sim, "elem"), 0x2f5, "nested element value");
    // Element [0][1] = high slot 10'h2FB = d='{4'hB, 4'hE}, m=2'b11;
    // d[1] is the high nibble of d = 4'hB.
    assert_eq!(u(&sim, "deep"), 0xb, "struct member lane through 2 selects");
}
