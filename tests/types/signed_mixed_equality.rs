//! Equality/`%0d` of integral operands where signedness meets an UNSIZED
//! literal, plus same-width class-member signedness.
//!
//! Reference-verified (§5.7.1 / §11.8):
//!  * An UNSIZED BASED literal keeps its NATURAL width: `'hfe` is 8 bits, so
//!    `byte b='hfe; b == 'hfe` is EQUAL and `b == 254` (an unsized DECIMAL =
//!    32-bit signed) is NOT equal. Previously `'hfe` was clamped to 32 bits,
//!    so `b` (signed, sign-extended to 0xFFFFFFFE) never matched the
//!    zero-extended `0x000000FE` — the failure behind a UVM resource_db
//!    byte-vs-int trace comparing `bvalue != 'hfe`.
//!  * An UNSIZED DECIMAL literal is at least 32 bits and signed, so
//!    `time t=-1; t == -1` is EQUAL (the 32-bit -1 sign-extends fully).
//!  * A class field's DECLARED signedness governs its stored value even on a
//!    same-width assignment: `byte f; f = 'haa` reads back -86, not 170
//!    (`fit_class_prop` now stamps `is_signed` from the member type on both
//!    the resize and width-match paths). Without it the UVM `uvm_transaction`
//!    table printer misprinted a signed field and its `accept_time != -1`
//!    gate (a `local time` member) flipped.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able (x/z?)", n))
        & 0xFFFF_FFFF
}

// ── Unsized based literal keeps natural width; decimal stays 32-bit ───
#[test]
fn unsized_based_literal_natural_width() {
    let src = r#"
module tb;
  byte b;
  int e1, e2, e3;
  initial begin
    b = 'hfe;
    e1 = (b == 'hfe);      // 'hfe is 8-bit -> 0xFE == 0xFE
    e2 = (b == 254);       // 254 is 32-bit signed -> -2 != 254
    e3 = (b == 8'shfe);    // both 8-bit
  end
endmodule
"#;
    let sim = simulate(src, 50).expect("simulate failed");
    assert_eq!(u(&sim, "e1"), 1, "unsized based 'hfe must be 8-bit (equal to byte 'hfe)");
    assert_eq!(u(&sim, "e2"), 0, "byte 'hfe vs unsized decimal 254 must differ");
    assert_eq!(u(&sim, "e3"), 1, "8'shfe equal to byte 'hfe");
}

// ── Unsized decimal literal sign-extends into a time comparison ───────
#[test]
fn time_minus_one_equals_signed_literal() {
    let src = r#"
module tb;
  time t;
  int e1, e2;
  initial begin
    t = -1;
    e1 = (t == -1);        // 32-bit -1 sign-extends to 0xFF..F
    e2 = (t == 32'hffffffff); // sized 32-bit unsigned, zero-extended -> differs
  end
endmodule
"#;
    let sim = simulate(src, 50).expect("simulate failed");
    assert_eq!(u(&sim, "e1"), 1, "time -1 == -1 (reference)");
    assert_eq!(u(&sim, "e2"), 0, "time -1 vs sized 32'hffffffff differs (reference)");
}

// ── Same-width class-member byte keeps DECLARED signedness ────────────
#[test]
fn same_width_byte_member_stays_signed() {
    let src = r#"
module tb;
  class holder;
    byte b;               // signed 8-bit
    function void load(); b = 'haa; endfunction
    function int less_zero(); return (b < 0); endfunction
  endclass
  holder h;
  int nz;
  initial begin
    h = new;
    h.load();             // b = 0xAA same-width (8-bit)
    nz = h.less_zero();   // signed byte -> negative
  end
endmodule
"#;
    let sim = simulate(src, 50).expect("simulate failed");
    // A `byte` member loaded with a same-width literal must stay signed.
    assert_eq!(u(&sim, "nz"), 1, "byte member 'haa must remain signed (negative)");
}
