//! IEEE 1800-2023 §11.4.11 conditional operator with ambiguous (x/z) select,
//! and §10.10.1 unpacked-array concatenation brace flattening.
//!
//! §11.4.11: when the `?:` condition is x/z, BOTH operands are evaluated and
//! merged bit by bit (Table 11-21): a result bit keeps the operands' value
//! only where both agree on 0 or 1; every other pairing (incl. z/z) gives x.
//!
//! §10.10.1: in an unpacked-array concatenation each item is read in the
//! context of the ELEMENT type, so a nested plain-brace concat is itself an
//! array value whose elements splice into the outer one — nested braces
//! flatten: `a = {1, {2, 3}}` is `a = {1, 2, 3}`. A queue/array operand
//! inside the concat likewise splices its elements (`q2 = {0, q, 9}`).

use xezim::simulate;

fn flag(sim: &xezim::compiler::Simulator, name: &str) -> u64 {
    sim.get_signal(name)
        .or_else(|| sim.get_signal(&format!("tb.{}", name)))
        .unwrap_or_else(|| panic!("signal not found: {}", name))
        .to_u64()
        .unwrap_or_else(|| panic!("signal {} has x/z bits", name))
}

/// §11.4.11: x select merges the operands per bit — agreeing bits keep their
/// value, differing bits become x. Includes a same-bits case where the two
/// operands are identical, so NO bit may degrade to x.
#[test]
fn xselect_merges_operands_bitwise() {
    const SRC: &str = r#"
module tb;
  logic sel = 1'bx;
  logic selz = 1'bz;
  logic [3:0] r_diff, r_same, r_z, r_zz;
  bit pass_diff, pass_same, pass_z, pass_zz;
  initial begin
    // bit3: 1/1 -> 1, bit2: 1/0 -> x, bit1: 0/1 -> x, bit0: 0/0 -> 0
    r_diff = sel ? 4'b1100 : 4'b1010;
    pass_diff = (r_diff === 4'b1xx0);
    // Operands agree on every bit: merge must NOT introduce any x.
    r_same = sel ? 4'b1010 : 4'b1010;
    pass_same = (r_same === 4'b1010);
    // A z select is ambiguous too (has_unknown covers x and z).
    r_z = selz ? 4'b1110 : 4'b1010;
    pass_z = (r_z === 4'b1x10);
    // Table 11-21: z merged with z gives x, not z.
    r_zz = sel ? 4'b110z : 4'b101z;
    pass_zz = (r_zz === 4'b1xxx);
  end
endmodule
"#;
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(flag(&sim, "pass_diff"), 1, "x-select must merge per bit: 1100/1010 -> 1xx0");
    assert_eq!(flag(&sim, "pass_same"), 1, "identical operands must merge with no x bits");
    assert_eq!(flag(&sim, "pass_z"), 1, "z select must merge like x: 1110/1010 -> 1x10");
    assert_eq!(flag(&sim, "pass_zz"), 1, "z/z bits must merge to x (Table 11-21)");
}

/// §10.10.1: nested plain braces in an unpacked-array concatenation flatten.
/// `{1, {2, 3}}` assigns `{1, 2, 3}` (and nesting flattens recursively).
/// Assignment patterns `'{...}` keep their non-flattening semantics for
/// arrays of arrays.
#[test]
fn nested_unpacked_concat_flattens() {
    const SRC: &str = r#"
module tb;
  int a[3];
  int d[3];
  int b[2][2];
  bit pass_a, pass_d, pass_b;
  initial begin
    a = {1, {2, 3}};
    pass_a = (a[0] == 1) && (a[1] == 2) && (a[2] == 3);
    // Nesting flattens recursively.
    d = {4, {5, {6}}};
    pass_d = (d[0] == 4) && (d[1] == 5) && (d[2] == 6);
    // Array-of-arrays uses assignment patterns '{...}: each inner pattern is
    // one whole sub-array element — must NOT be flattened.
    b = '{'{1, 2}, '{3, 4}};
    pass_b = (b[0][0] == 1) && (b[0][1] == 2) && (b[1][0] == 3) && (b[1][1] == 4);
  end
endmodule
"#;
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(flag(&sim, "pass_a"), 1, "a = {{1, {{2, 3}}}} must flatten to {{1, 2, 3}}");
    assert_eq!(flag(&sim, "pass_d"), 1, "nested braces must flatten recursively");
    assert_eq!(flag(&sim, "pass_b"), 1, "'{{...}} sub-array patterns must not flatten");
}

/// §10.10.1: a queue/array operand inside an unpacked concatenation splices
/// its elements — `q2 = {0, q, 9}` — and still does so through flattening
/// (`{0, {q, 9}}`).
#[test]
fn queue_splice_in_concat_preserved() {
    const SRC: &str = r#"
module tb;
  int q[$];
  int q2[$];
  int q3[$];
  bit pass_q2, pass_q3;
  initial begin
    q = {5, 6};
    q2 = {0, q, 9};
    pass_q2 = (q2.size() == 4) && (q2[0] == 0) && (q2[1] == 5)
              && (q2[2] == 6) && (q2[3] == 9);
    // Splice still works for a queue operand inside a nested brace.
    q3 = {0, {q, 9}};
    pass_q3 = (q3.size() == 4) && (q3[0] == 0) && (q3[1] == 5)
              && (q3[2] == 6) && (q3[3] == 9);
  end
endmodule
"#;
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(flag(&sim, "pass_q2"), 1, "queue operand in concat must splice its elements");
    assert_eq!(flag(&sim, "pass_q3"), 1, "queue splice must survive nested-brace flattening");
}
