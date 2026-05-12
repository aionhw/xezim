//! SystemVerilog logical implication `->`, equivalence `<->`, wildcard
//! equality `==?` / `!=?`, and the `is_nonzero` X-propagation fix.
//!
//! Regression for: xezim-core 12ed5c9 (-> / <-> parser + Value methods),
//! xezim a88d5b8 (-> / <-> eval), xezim-core fd2e059 + xezim 2f6b201
//! (==? / !=?), xezim-core 004f5b2 (is_nonzero: definite-1 ⇒ truthy).

use xezim::simulate;

fn u64_of(sim: &xezim::compiler::Simulator, name: &str) -> u64 {
    for n in [name.to_string(), format!("tb.{name}")] {
        if let Some(v) = sim.get_signal(&n) {
            return v.to_u64().unwrap_or_else(|| panic!("{n} has X/Z, expected defined")) & 0xFFFF_FFFF;
        }
    }
    panic!("signal not found: {name}");
}

const SRC: &str = r#"
module tb;
  reg a, b;
  reg [3:0] x;
  reg [3:0] yw;            // a 4-state value: 4'b1x1x
  reg r_i00, r_i01, r_i10, r_i11;
  reg r_e00, r_e01, r_e10, r_e11;
  reg r_i_nonbool;
  reg r_w_eq, r_w_ne, r_w_wild, r_w_mismatch;
  reg r_xprop_is_one;

  initial begin
    yw = 4'b1x1x;

    a = 0; b = 0; #1; r_i00 = (a -> b); r_e00 = (a <-> b);
    a = 0; b = 1; #1; r_i01 = (a -> b); r_e01 = (a <-> b);
    a = 1; b = 0; #1; r_i10 = (a -> b); r_e10 = (a <-> b);
    a = 1; b = 1; #1; r_i11 = (a -> b); r_e11 = (a <-> b);

    x = 4'b1010; #1; r_i_nonbool = ((|x) -> x[1]);

    x = 4'b1010; #1; r_w_eq = (x ==? 4'b1010);
    x = 4'b1010; #1; r_w_ne = (x !=? 4'b1011);
    x = 4'b1011; #1; r_w_wild = (x ==? yw);
    x = 4'b0011; #1; r_w_mismatch = (x ==? yw);

    x = 4'b1xxx; #1; r_xprop_is_one = ((x && 1'b1) === 1'b1);

    $finish;
  end
endmodule
"#;

#[test]
fn implication_truth_table() {
    let sim = simulate(SRC, 200).expect("simulate failed");
    assert_eq!(u64_of(&sim, "r_i00"), 1, "0 -> 0 should be 1");
    assert_eq!(u64_of(&sim, "r_i01"), 1, "0 -> 1 should be 1");
    assert_eq!(u64_of(&sim, "r_i10"), 0, "1 -> 0 should be 0");
    assert_eq!(u64_of(&sim, "r_i11"), 1, "1 -> 1 should be 1");
    assert_eq!(u64_of(&sim, "r_i_nonbool"), 1, "(|4'b1010) -> 4'b1010[1] should be 1");
}

#[test]
fn equivalence_truth_table() {
    let sim = simulate(SRC, 200).expect("simulate failed");
    assert_eq!(u64_of(&sim, "r_e00"), 1, "0 <-> 0 should be 1");
    assert_eq!(u64_of(&sim, "r_e01"), 0, "0 <-> 1 should be 0");
    assert_eq!(u64_of(&sim, "r_e10"), 0, "1 <-> 0 should be 0");
    assert_eq!(u64_of(&sim, "r_e11"), 1, "1 <-> 1 should be 1");
}

#[test]
fn wildcard_equality() {
    let sim = simulate(SRC, 200).expect("simulate failed");
    assert_eq!(u64_of(&sim, "r_w_eq"), 1, "4'b1010 ==? 4'b1010 should be 1");
    assert_eq!(u64_of(&sim, "r_w_ne"), 1, "4'b1010 !=? 4'b1011 should be 1");
    assert_eq!(u64_of(&sim, "r_w_wild"), 1, "4'b1011 ==? 4'b1x1x should be 1 (rhs x = wildcard)");
    assert_eq!(u64_of(&sim, "r_w_mismatch"), 0, "4'b0011 ==? 4'b1x1x should be 0 (hard mismatch on bit 3)");
}

#[test]
fn x_propagation_definite_one() {
    let sim = simulate(SRC, 200).expect("simulate failed");
    assert_eq!(
        u64_of(&sim, "r_xprop_is_one"), 1,
        "4'b1xxx && 1'b1 should be 1 (definite-1 => truthy), not X"
    );
}
