//! IEEE 1800-2017 §6.20.2: an UNPACKED-ARRAY parameter.
//!
//!   module m #(parameter int N = 1, u32_t START_ADDR[N] = {32'h0}) ...
//!   m #(.N(2), .START_ADDR({32'he000_0000, 32'he000_1000})) u (...);
//!
//! Three separate defects made `START_ADDR[i]` read 0:
//!   1. The parameter port list only split on KEYWORD types at a comma, so
//!      `, u32_t START_ADDR` parsed as another assignment named `u32_t`.
//!   2. No element signals were ever created — and an override arrives already
//!      collapsed to one packed value, so it never reached an element.
//!   3. Without an `arrays` entry, `START_ADDR[i]` was a bit-select of a
//!      nonexistent scalar rather than an element select.

use xezim::simulate;

const SRC: &str = r#"
package P;
  typedef logic [31:0] u32_t;
endpackage

module dut import P::*; #(parameter int N = 1,
                          u32_t START_ADDR[N] = {32'h0},
                          u32_t END_ADDR[N]   = {32'hFFFF_FFFF}) ();
  int s0, s1, e0, e1;
  int hit_lo, hit_hi, miss;
  initial begin
    s0 = START_ADDR[0]; s1 = START_ADDR[1];
    e0 = END_ADDR[0];   e1 = END_ADDR[1];
    // The elements must be usable as `inside` range bounds.
    hit_lo = (32'he000_00ad inside { [START_ADDR[0]:END_ADDR[0]] });
    hit_hi = (32'he000_10ad inside { [START_ADDR[1]:END_ADDR[1]] });
    miss   = (32'he000_0f00 inside { [START_ADDR[0]:END_ADDR[0]] });
  end
endmodule

module tb;
  dut #(.N(2),
        .START_ADDR({32'he000_0000, 32'he000_1000}),
        .END_ADDR({32'he000_0100, 32'he000_1100})) u ();
endmodule
"#;

/// The default (non-overridden) initializer must materialize too.
const DEFAULTED: &str = r#"
package P;
  typedef logic [31:0] u32_t;
endpackage

module dut import P::*; #(parameter int N = 2,
                          u32_t BASE[N] = {32'hAAAA_0000, 32'hBBBB_0000}) ();
  int b0, b1;
  initial begin
    b0 = BASE[0];
    b1 = BASE[1];
  end
endmodule

module tb;
  dut u ();
endmodule
"#;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able", n))
        & 0xFFFF_FFFF
}

#[test]
fn overridden_array_parameter_reaches_its_elements() {
    let sim = simulate(SRC, 100).expect("simulate failed");

    // `{a, b}` puts `a` in the high bits, and `a` is element 0.
    assert_eq!(u(&sim, "u.s0"), 0xe000_0000);
    assert_eq!(u(&sim, "u.s1"), 0xe000_1000);
    assert_eq!(u(&sim, "u.e0"), 0xe000_0100);
    assert_eq!(u(&sim, "u.e1"), 0xe000_1100);
}

#[test]
fn array_parameter_elements_work_as_inside_bounds() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "u.hit_lo"), 1);
    assert_eq!(u(&sim, "u.hit_hi"), 1);
    assert_eq!(u(&sim, "u.miss"), 0);
}

#[test]
fn default_array_parameter_initializer_materializes() {
    let sim = simulate(DEFAULTED, 100).expect("simulate failed");
    assert_eq!(u(&sim, "u.b0"), 0xAAAA_0000);
    assert_eq!(u(&sim, "u.b1"), 0xBBBB_0000);
}
