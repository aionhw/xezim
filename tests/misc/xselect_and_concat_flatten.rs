//! IEEE 1800-2023 §11.4.11 conditional operator with ambiguous (x/z) select,
//! and §10.10.3 nested element expressions in unpacked concatenations.

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
    assert_eq!(
        flag(&sim, "pass_diff"),
        1,
        "x-select must merge per bit: 1100/1010 -> 1xx0"
    );
    assert_eq!(
        flag(&sim, "pass_same"),
        1,
        "identical operands must merge with no x bits"
    );
    assert_eq!(
        flag(&sim, "pass_z"),
        1,
        "z select must merge like x: 1110/1010 -> 1x10"
    );
    assert_eq!(
        flag(&sim, "pass_zz"),
        1,
        "z/z bits must merge to x (Table 11-21)"
    );
}

/// §10.10.3: nested braces contribute one packed or string value.
#[test]
fn unpacked_concat_keeps_nested_element_expressions() {
    const SRC: &str = r#"
module tb;
  logic [7:0] fixed_array[2], dynamic_array[];
  logic [63:0] q[$];
  int rows[2][2];
  logic [7:0] packed_rows[2][2];
  string words[$], ending;
  typedef logic [7:0] small_queue_t[$];
  small_queue_t returned;
  small_queue_t initialized = {8'h10, {4'h2, 4'h3}};
  bit pass_fixed, pass_dynamic, pass_queue, pass_return, pass_init, pass_strings, pass_rows, pass_selected;

  function automatic small_queue_t make_queue();
    return {8'h10, {4'h2, {2'b00, 2'b11}}};
  endfunction

  initial begin
    fixed_array = {8'h10, {4'h2, 4'h3}};
    dynamic_array = {fixed_array, {4'h4, 4'h5}};
    q = {q, {32'h8000_0080, 32'h03d0_01db}};
    q = {q, {32'h8000_0084, 32'h2002_05b7}};
    returned = make_queue();
    ending = "world";
    words = {"hello", {" ", ending}};
    rows = '{'{1, 2}, '{3, 4}};
    packed_rows[1] = {8'h99, {4'h2, 4'h3}};

    pass_fixed = fixed_array[0] == 8'h10 && fixed_array[1] == 8'h23;
    pass_dynamic = dynamic_array.size() == 3 && dynamic_array[2] == 8'h45;
    pass_queue = q.size() == 2 && q[0] == 64'h80000080_03d001db
                               && q[1] == 64'h80000084_200205b7;
    pass_return = returned.size() == 2 && returned[1] == 8'h23;
    pass_init = initialized.size() == 2 && initialized[1] == 8'h23;
    pass_strings = words.size() == 2 && words[0] == "hello" && words[1] == " world";
    pass_selected = packed_rows[1][0] == 8'h99 && packed_rows[1][1] == 8'h23;
    pass_rows = rows[0][0] == 1 && rows[0][1] == 2 && rows[1][0] == 3 && rows[1][1] == 4;
  end
endmodule
"#;
    let sim = simulate(SRC, 100).expect("simulate failed");
    for name in ["pass_fixed", "pass_dynamic", "pass_queue", "pass_return",
                 "pass_init", "pass_strings", "pass_rows", "pass_selected"] {
        assert_eq!(flag(&sim, name), 1, "{name}");
    }
}

#[test]
fn continuous_unpacked_concat_drives_nested_packed_element() {
    let sim = simulate(
        r#"
module tb;
  logic [3:0] hi = 4'h2, lo = 4'h3;
  wire [7:0] a[2];
  assign a = {8'h10, {hi, lo}};
  bit pass_first, pass_second;
  initial begin
    #1 pass_first = a[0] == 8'h10 && a[1] == 8'h23;
    hi = 4'h4;
    #1 pass_second = a[0] == 8'h10 && a[1] == 8'h43;
  end
endmodule
"#,
        100,
    )
    .expect("simulate failed");
    assert_eq!(flag(&sim, "pass_first"), 1);
    assert_eq!(flag(&sim, "pass_second"), 1);
}
