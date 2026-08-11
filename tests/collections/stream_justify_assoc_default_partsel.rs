//! An LRM sweep over chapters 7 / 11 / 21 — six defects, all reference-checked.
//!
//! 1. **§11.4.14.2 stream justification.** A streaming concatenation assigned
//!    to a WIDER integral target is LEFT-justified. The LRM's own example is
//!    `bit [99:0] d = {>>{a, b, c}};  // d[3:0] = 0` — a 96-bit stream landing
//!    in `d[99:4]` — and the companion `int j = {>>{a, b, c}};` is called an
//!    ERROR precisely because a stream is never truncated. xezim zero-extended
//!    on the LEFT like an ordinary assignment, so every value came out shifted
//!    into the low half.
//! 2. The same rule under a CAST: `int'({<<8{16'hABCD}})` is `32'hcdab_0000`.
//!    The cast evaluated its operand self-determined, so the width never
//!    reached the stream.
//! 3. **§7.8.6 nonexistent associative-array element.** A read yields the
//!    element type's default — 0 for a 2-state type, x for a 4-state one — at
//!    the ELEMENT's width. xezim returned a flat 1-bit x for every type, so
//!    `int aa[int]; aa[7]` read x instead of 0.
//! 4. **§6.12.2 real arrays.** The "bit-select of a real is illegal" check
//!    keyed on "the signal is real" alone, so `r[i]` on an ARRAY of reals was
//!    rejected and any module declaring one failed to elaborate.
//! 5. **§11.5.1 negative part-select bounds.** `w[-4 +: 8]` read its bounds
//!    through `to_u64`, turning -4 into a huge unsigned index; the range guard
//!    in `get_bit` then TRUNCATED the index to u32, let it through, and the
//!    shift panicked. Out-of-range bits must simply read x.
//! 6. **§6.16.3 string relational operators** compared zero-extended bit
//!    vectors, which makes LENGTH dominate: `"Jello" < "z"` was false because
//!    "z" widened to 0x000000007A.
//!
//! Also here: `%5s` applied the field width to a string VARIABLE but dropped
//! it for a string LITERAL.
//!
//! Radix field padding without the `0` flag (`%4h` of 8'h0f): two commercial
//! simulators disagree — space-pad to the natural width ("  0f") vs trim to
//! the minimal form and zero-pad ("000f"). Originally left as-was; since
//! resolved in favour of the reference simulator's minimal+zero-pad model
//! (G8 audit fix, pinned in `format_sibling_fixes.rs`).

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("top.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able (x/z?)", n))
}

fn outs(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

fn line(sim: &xezim::compiler::Simulator, tag: &str) -> String {
    let o = outs(sim);
    o.iter()
        .find(|s| s.starts_with(tag))
        .unwrap_or_else(|| panic!("missing {tag}:\n{}", o.join("\n")))
        .clone()
}

/// A stream into a wider target is left-justified, in an assignment and under
/// a cast; an exactly-sized target is unaffected.
#[test]
fn stream_into_a_wider_target_is_left_justified() {
    let src = r#"
module top;
  logic [31:0] w_fwd, w_rev, w_nib, w_bit;
  logic [15:0] exact;
  int cast_rev, cast_fwd;
  bit [63:0] big;
  initial begin
    w_fwd = {>>{16'hABCD}};
    w_rev = {<<8{16'hABCD}};
    w_nib = {<<4{16'hABCD}};
    w_bit = {<<{8'b0000_0001}};
    exact = {<<4{16'hABCD}};
    cast_rev = int'({<<8{16'hABCD}});
    cast_fwd = int'({>>8{16'hABCD}});
    big = {>>{32'h1234_5678}};
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(u(&sim, "w_fwd"), 0xABCD_0000, "a plain stream, wider target");
    assert_eq!(u(&sim, "w_rev"), 0xCDAB_0000, "byte-reversed");
    assert_eq!(u(&sim, "w_nib"), 0xDCBA_0000, "nibble-reversed");
    assert_eq!(u(&sim, "w_bit"), 0x8000_0000, "bit-reversed");
    assert_eq!(u(&sim, "exact"), 0xDCBA, "an exactly-sized target is unchanged");
    assert_eq!(u(&sim, "cast_rev"), 0xCDAB_0000, "the cast width reaches the stream");
    assert_eq!(u(&sim, "cast_fwd"), 0xABCD_0000);
    assert_eq!(u(&sim, "big"), 0x1234_5678_0000_0000, "64-bit target");
}

/// The unpack direction (stream as the assignment TARGET) and dynamic-array
/// targets keep working — those never went through the widening path.
#[test]
fn stream_unpack_and_dynamic_targets_are_unchanged() {
    let src = r#"
module top;
  logic [31:0] w;
  logic [7:0]  b[4];
  byte         d[];
  logic [31:0] repacked;
  int b0, b1, b2, b3, r0, r3;
  initial begin
    w = 32'hAABBCCDD;
    {>>{b}} = w;
    b0 = b[0]; b1 = b[1]; b2 = b[2]; b3 = b[3];
    d = new[4];
    {>>{d}} = 32'h11223344;
    r0 = d[0]; r3 = d[3];
    repacked = {>>{d}};
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!((u(&sim, "b0"), u(&sim, "b3")), (0xAA, 0xDD), "unpack into a fixed array");
    assert_eq!(u(&sim, "b1"), 0xBB);
    assert_eq!(u(&sim, "b2"), 0xCC);
    assert_eq!((u(&sim, "r0"), u(&sim, "r3")), (0x11, 0x44), "unpack into a dynamic array");
    assert_eq!(u(&sim, "repacked"), 0x1122_3344, "and repack out of it");
}

/// §7.8.6: the default value follows the ELEMENT TYPE, and reading does not
/// create the element.
#[test]
fn nonexistent_assoc_element_reads_the_element_type_default() {
    let src = r#"
module top;
  int         i_aa[int];
  bit  [7:0]  b_aa[int];
  byte        y_aa[int];
  logic [7:0] l_aa[int];
  integer     g_aa[int];
  int         w_aa[*];
  int i_def, b_def, y_def, w_def, n_after, present, absent;
  initial begin
    i_def = i_aa[7];
    b_def = b_aa[7];
    y_def = y_aa[7];
    w_def = w_aa[7];
    n_after = i_aa.num() + l_aa.num();
    $display("L=%h G=%0d", l_aa[7], g_aa[7]);
    i_aa[1] = 5;
    present = i_aa[1];
    absent  = i_aa[2];
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(u(&sim, "i_def"), 0, "int: 2-state default 0");
    assert_eq!(u(&sim, "b_def"), 0, "bit [7:0]: 0");
    assert_eq!(u(&sim, "y_def"), 0, "byte: 0");
    assert_eq!(u(&sim, "w_def"), 0, "wildcard-indexed int: 0");
    assert_eq!(u(&sim, "n_after"), 0, "reading does not create the element");
    assert_eq!(u(&sim, "present"), 5);
    assert_eq!(u(&sim, "absent"), 0);
    // 4-state stays x, at the ELEMENT width — a 1-bit x printed "x" for what
    // must be "xx".
    assert_eq!(line(&sim, "L="), "L=xx G=x", "4-state defaults are x, full width");
}

/// §6.12.2: an array OF reals is indexable; a real SCALAR bit-select is still
/// rejected.
#[test]
fn arrays_of_reals_are_indexable() {
    let src = r#"
module top;
  real fa[4];
  real dq[$];
  real da[];
  real aa[string];
  int ok_fixed, ok_queue, ok_dyn, ok_assoc, ok_default;
  initial begin
    fa[0] = 1.5;
    dq.push_back(3.5);
    da = new[2]; da[1] = 9.75;
    aa["k"] = 0.125;
    ok_fixed  = (fa[0] == 1.5);
    ok_queue  = (dq[0] == 3.5);
    ok_dyn    = (da[1] == 9.75);
    ok_assoc  = (aa["k"] == 0.125);
    ok_default = (aa["nope"] == 0.0);
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(u(&sim, "ok_fixed"), 1, "fixed array of reals");
    assert_eq!(u(&sim, "ok_queue"), 1, "queue of reals");
    assert_eq!(u(&sim, "ok_dyn"), 1, "dynamic array of reals");
    assert_eq!(u(&sim, "ok_assoc"), 1, "associative array of reals");
    assert_eq!(u(&sim, "ok_default"), 1, "and its default element is 0.0");

    let bad = r#"
module top;
  real r;
  logic b;
  initial begin r = 1.5; b = r[0]; end
endmodule
"#;
    let err = match simulate(bad, 20) {
        Ok(_) => panic!("a real scalar bit-select must still be rejected"),
        Err(e) => e.to_string(),
    };
    assert!(err.contains("Bit-select of real"), "got: {err}");
}

/// §11.5.1: a negative part-select base reads x for the out-of-range bits
/// instead of panicking.
#[test]
fn negative_part_select_bounds_read_x() {
    let src = r#"
module top;
  logic [31:0] w;
  logic [7:0] neg_up, oob_up, in_range;
  logic [3:0] neg_dn;
  initial begin
    w = 32'h89ABCDEF;
    neg_up   = w[-4 +: 8];
    oob_up   = w[28 +: 8];
    neg_dn   = w[2 -: 4];
    in_range = w[8 +: 8];
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    let neg = sim.get_signal("top.neg_up").or_else(|| sim.get_signal("neg_up")).expect("neg_up");
    // bits 3..0 are 'f', bits -1..-4 are x
    assert_eq!(neg.to_hex_string(), "fx", "w[-4 +: 8] is fx, not a panic");
    let oob = sim.get_signal("top.oob_up").or_else(|| sim.get_signal("oob_up")).expect("oob_up");
    assert_eq!(oob.to_hex_string(), "x8", "above the MSB reads x");
    let dn = sim.get_signal("top.neg_dn").or_else(|| sim.get_signal("neg_dn")).expect("neg_dn");
    assert_eq!(
        dn.to_hex_string(),
        "X",
        "the -: form: bits 2..0 are 1 and bit -1 is x, so the single nibble is X"
    );
    assert_eq!(u(&sim, "in_range"), 0xCD, "an ordinary indexed part-select");
}

/// §6.16.3: string ordering is lexicographic, not by packed value.
#[test]
fn string_relational_operators_compare_lexicographically() {
    let src = r#"
module top;
  string a, b;
  int lt1, gt1, le1, ge1, lt2, lt3, lt4, lt5, eq1;
  initial begin
    a = "Jello"; b = "z";
    lt1 = a < b; gt1 = a > b; le1 = a <= b; ge1 = a >= b;
    a = "abc"; b = "abd";  lt2 = a < b;
    a = "abc"; b = "abcd"; lt3 = a < b;    // a prefix is smaller
    a = "";    b = "a";    lt4 = a < b;
    a = "B";   b = "a";    lt5 = a < b;    // uppercase sorts first
    a = "abc"; b = "abc";  eq1 = (a == b) && (a <= b) && (a >= b) && !(a < b);
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(u(&sim, "lt1"), 1, "\"Jello\" < \"z\" — length must not dominate");
    assert_eq!(u(&sim, "gt1"), 0);
    assert_eq!(u(&sim, "le1"), 1);
    assert_eq!(u(&sim, "ge1"), 0);
    assert_eq!(u(&sim, "lt2"), 1, "differing at the last character");
    assert_eq!(u(&sim, "lt3"), 1, "a prefix compares less");
    assert_eq!(u(&sim, "lt4"), 1, "the empty string is smallest");
    assert_eq!(u(&sim, "lt5"), 1, "'B'(0x42) < 'a'(0x61)");
    assert_eq!(u(&sim, "eq1"), 1, "equality and the non-strict forms agree");
}

/// `%s` honours its field width for a literal, as it already did for a
/// variable.
#[test]
fn string_format_width_applies_to_literals() {
    let src = r#"
module top;
  string s;
  initial begin
    s = "ab";
    $display("VAR|%5s|%-5s|%0s|", s, s, s);
    $display("LIT|%5s|%-5s|%0s|", "ab", "ab", "ab");
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    let v = line(&sim, "VAR");
    let l = line(&sim, "LIT");
    assert_eq!(&v[3..], "|   ab|ab   |ab|", "variable");
    assert_eq!(&l[3..], "|   ab|ab   |ab|", "literal — same treatment");
}

/// §11.5.1 partial-WRITE through an out-of-range part-select — the four
/// defects behind the only ivtest cases that ABORTED the simulator
/// (`pv_wr_vec2*`). All four expectations are confirmed by a reference
/// simulator, which passes those tests.
///
/// * `x[4:-1] = …` computed `l + r - 1` on a wrapped unsigned -1 and aborted;
/// * the whole-word fast path then fired on the CLAMPED bounds and stored the
///   wrong bits — a negative low bound SHIFTS which source bit lands where;
/// * `infer_lhs_width` returned 0 for `x[0:-1]` (the u64 -1 made `l >= r`
///   pick the wrong branch), so the NBA form resized its value to nothing;
/// * an x/z index wrote at bit 0 instead of being discarded — twice over,
///   since the NBA path first FROZE the index through `to_i64`, which masks
///   the unknown bits away before the write path can see them.
#[test]
fn partial_writes_through_out_of_range_part_selects() {
    let src = r#"
`timescale 1ns/1ns
module top;
  bit [3:0] x;
  integer i;
  int b_both, b_low, b_high, b_oob_low, b_oob_high, b_xidx;
  int n_both, n_low, n_high, n_oob_low, n_xidx;
  int v_both, v_low, v_xidx;
  initial begin
    // blocking
    x = 0; x[4:-1]  = 6'b101010; b_both     = x;
    x = 0; x[0:-1]  = 2'b10;     b_low      = x;
    x = 0; x[4:3]   = 2'b01;     b_high     = x;
    x = 0; x[-1:-2] = 2'b11;     b_oob_low  = x;
    x = 0; x[6:5]   = 2'b11;     b_oob_high = x;
    i = 'hx; x = 0; x[i +: 2] = 2'b11; b_xidx = x;
    // nonblocking
    x = 0; x[4:-1]  <= 6'b101010; #1 n_both    = x;
    x = 0; x[0:-1]  <= 2'b10;     #1 n_low     = x;
    x = 0; x[4:3]   <= 2'b01;     #1 n_high    = x;
    x = 0; x[-1:-2] <= 2'b11;     #1 n_oob_low = x;
    i = 'hx; x = 0; x[i +: 2] <= 2'b11; #1 n_xidx = x;
    // variable base, indexed form
    i = -1; x = 0; x[i +: 6] = 6'b101010; v_both = x;
    i = -1; x = 0; x[i +: 2] = 2'b10;     v_low  = x;
    i = 'hx; x = 0; x[i +: 2] = 2'b11;    v_xidx = x;
  end
endmodule
"#;
    let sim = simulate(src, 200).expect("simulate failed");
    // Out at BOTH ends: source bits 4..1 land in x[3:0] -> 0101, not 1010.
    assert_eq!(u(&sim, "b_both"), 0b0101, "blocking, both ends out of range");
    assert_eq!(u(&sim, "b_low"), 0b0001, "blocking, low end out");
    assert_eq!(u(&sim, "b_high"), 0b1000, "blocking, high end out");
    assert_eq!(u(&sim, "b_oob_low"), 0, "entirely below bit 0: no write");
    assert_eq!(u(&sim, "b_oob_high"), 0, "entirely above the MSB: no write");
    assert_eq!(u(&sim, "b_xidx"), 0, "an x index discards the write");
    assert_eq!(u(&sim, "n_both"), 0b0101, "NBA, both ends out of range");
    assert_eq!(u(&sim, "n_low"), 0b0001, "NBA width must not collapse to 0");
    assert_eq!(u(&sim, "n_high"), 0b1000);
    assert_eq!(u(&sim, "n_oob_low"), 0);
    assert_eq!(u(&sim, "n_xidx"), 0, "not even after index freezing");
    assert_eq!(u(&sim, "v_both"), 0b0101, "variable base, indexed form");
    assert_eq!(u(&sim, "v_low"), 0b0001);
    assert_eq!(u(&sim, "v_xidx"), 0);
}
