//! §7.4.5, §10.4, §11.6.1: an element of a multidimensional unpacked array is
//! as wide as its declared type, and that width is the context of a value
//! stored into it. A process compiled to bytecode took 32 bits as the width of
//! `a[i][j]`, so `a[0][0] <= v` dropped everything above bit 31. Expected
//! values are the reference simulator's.

use xezim::{simulate, simulate_multi};

const V: u64 = 0xFEDC_BA98_7654_3210;

/// `(name, value)` for each expected signal, under a named top and under the
/// top the simulator finds itself.
#[track_caller]
fn expect(src: &str, want: &[(&str, u64)]) {
    let named = simulate_multi(
        &[src.to_string()],
        1000,
        Some("tb"),
        &[],
        &[],
        None,
        false,
        None,
        None,
        &[],
        &[],
        None,
        &[],
        0,
        u64::MAX,
        None,
        &[],
        None,
        None,
        None,
        None,
        false,
    )
    .expect("simulate failed");
    let found = simulate(src, 1000).expect("simulate failed");
    for (how, sim) in [("named top", named), ("found top", found)] {
        for (name, value) in want {
            let got = sim.get_signal(name).and_then(|v| v.to_u64());
            assert_eq!(
                got,
                Some(*value),
                "{name} ({how}): got {got:x?}, want {value:x}"
            );
        }
    }
}

#[test]
fn nonblocking_store_with_constant_indices() {
    expect(
        r#"module tb;
  logic clk = 0; always #5 clk = ~clk;
  logic [63:0] v = 64'hFEDC_BA98_7654_3210;
  logic [63:0] a [1:0][1:0];
  logic [63:0] r;
  always_ff @(posedge clk) a[0][1] <= v;
  initial begin repeat (2) @(posedge clk); #1 r = a[0][1]; $finish; end
endmodule"#,
        &[("r", V)],
    );
}

#[test]
fn nonblocking_store_with_variable_indices() {
    expect(
        r#"module tb;
  logic clk = 0; always #5 clk = ~clk;
  logic [63:0] v = 64'hFEDC_BA98_7654_3210;
  logic [63:0] a [1:0][1:0];
  logic [63:0] r;
  int i = 1, j = 0;
  always_ff @(posedge clk) a[i][j] <= v;
  initial begin repeat (2) @(posedge clk); #1 r = a[1][0]; $finish; end
endmodule"#,
        &[("r", V)],
    );
}

#[test]
fn blocking_store_in_a_process_that_waits_on_a_clock() {
    expect(
        r#"module tb;
  logic clk = 0; always #5 clk = ~clk;
  logic [63:0] v = 64'hFEDC_BA98_7654_3210;
  logic [63:0] a [1:0][1:0];
  logic [63:0] r;
  initial begin a[0][0] = v; @(posedge clk); r = a[0][0]; $finish; end
endmodule"#,
        &[("r", V)],
    );
}

#[test]
fn every_element_keeps_its_own_value() {
    expect(
        r#"module tb;
  logic clk = 0; always #5 clk = ~clk;
  logic [39:0] a [1:0][1:0];
  logic [39:0] r00, r01, r10, r11;
  always_ff @(posedge clk) begin
    a[0][0] <= 40'hA0_0000_0000;
    a[0][1] <= 40'hA1_0000_0001;
    a[1][0] <= 40'hA2_0000_0002;
    a[1][1] <= 40'hA3_0000_0003;
  end
  initial begin
    repeat (2) @(posedge clk);
    #1 r00 = a[0][0]; r01 = a[0][1]; r10 = a[1][0]; r11 = a[1][1];
    $finish;
  end
endmodule"#,
        &[
            ("r00", 0xA0_0000_0000),
            ("r01", 0xA1_0000_0001),
            ("r10", 0xA2_0000_0002),
            ("r11", 0xA3_0000_0003),
        ],
    );
}

#[test]
fn ranges_that_do_not_start_at_zero_or_ascend() {
    expect(
        r#"module tb;
  logic clk = 0; always #5 clk = ~clk;
  logic [63:0] v = 64'hFEDC_BA98_7654_3210;
  logic [63:0] offset [3:2][5:4];
  logic [63:0] ascending [0:1][0:1];
  logic [63:0] r1, r2;
  always_ff @(posedge clk) begin offset[3][4] <= v; ascending[0][1] <= v; end
  initial begin
    repeat (2) @(posedge clk);
    #1 r1 = offset[3][4]; r2 = ascending[0][1];
    $finish;
  end
endmodule"#,
        &[("r1", V), ("r2", V)],
    );
}

#[test]
fn element_wider_than_64_bits() {
    expect(
        r#"module tb;
  logic clk = 0; always #5 clk = ~clk;
  logic [99:0] a [1:0][1:0];
  logic [99:0] e;
  logic [35:0] hi;
  logic [63:0] lo;
  always_ff @(posedge clk) a[1][0] <= {36'hABC_DEF_012, 64'hFEDC_BA98_7654_3210};
  initial begin
    repeat (2) @(posedge clk);
    #1 e = a[1][0]; hi = e[99:64]; lo = e[63:0];
    $finish;
  end
endmodule"#,
        &[("hi", 0xA_BCDE_F012), ("lo", V)],
    );
}

#[test]
fn packed_struct_and_two_state_elements() {
    expect(
        r#"package q;
  typedef struct packed { logic [31:0] pc; logic [7:0] m; } ent_t;
endpackage
module tb;
  logic clk = 0; always #5 clk = ~clk;
  q::ent_t s [1:0][1:0];
  bit [63:0] b [1:0][1:0];
  logic [31:0] pc;
  logic [7:0] m;
  logic [63:0] r;
  always_ff @(posedge clk) begin s[0][0] <= 40'hDE_AD00_00A5; b[0][0] <= 64'hFEDC_BA98_7654_3210; end
  initial begin
    repeat (2) @(posedge clk);
    #1 pc = s[0][0].pc; m = s[0][0].m; r = b[0][0];
    $finish;
  end
endmodule"#,
        &[("pc", 0xDEAD_0000), ("m", 0xA5), ("r", V)],
    );
}

#[test]
fn always_comb_and_continuous_assign() {
    expect(
        r#"module tb;
  logic [63:0] v = 64'hFEDC_BA98_7654_3210;
  logic [63:0] c [1:0][1:0];
  logic [63:0] d [1:0][1:0];
  logic [63:0] r1, r2;
  always_comb c[0][1] = v;
  assign d[1][1] = v;
  initial begin #1 r1 = c[0][1]; r2 = d[1][1]; $finish; end
endmodule"#,
        &[("r1", V), ("r2", V)],
    );
}

#[test]
fn three_dimensions() {
    expect(
        r#"module tb;
  logic clk = 0; always #5 clk = ~clk;
  logic [63:0] v = 64'hFEDC_BA98_7654_3210;
  logic [63:0] c [1:0][1:0][1:0];
  logic [63:0] d [1:0][1:0][1:0];
  logic [63:0] r1, r2;
  int i = 1, j = 0, k = 1;
  always_ff @(posedge clk) begin c[1][0][1] <= v; d[i][j][k] <= v; end
  initial begin
    repeat (2) @(posedge clk);
    #1 r1 = c[1][0][1]; r2 = d[1][0][1];
    $finish;
  end
endmodule"#,
        &[("r1", V), ("r2", V)],
    );
}

#[test]
fn the_element_is_the_context_of_the_value() {
    // §11.6.1: the sum of two 32-bit values keeps its carry in a 40-bit target.
    expect(
        r#"module tb;
  logic clk = 0; always #5 clk = ~clk;
  logic [31:0] x = 32'hFFFF_FFFF, y = 32'h1;
  logic signed [15:0] n = -1;
  logic [39:0] sum [1:0][1:0];
  logic signed [39:0] ext [1:0][1:0];
  logic [39:0] r1, r2;
  always_ff @(posedge clk) begin sum[0][0] <= x + y; ext[0][0] <= n; end
  initial begin
    repeat (2) @(posedge clk);
    #1 r1 = sum[0][0]; r2 = ext[0][0];
    $finish;
  end
endmodule"#,
        &[("r1", 0x1_0000_0000), ("r2", 0xFF_FFFF_FFFF)],
    );
}

#[test]
fn compound_and_increment_in_a_clocked_process() {
    expect(
        r#"module tb;
  logic clk = 0; always #5 clk = ~clk;
  logic [63:0] c [1:0][1:0];
  logic [63:0] d [1:0][1:0];
  logic [63:0] r1, r2;
  always @(posedge clk) begin
    c[0][0] = 64'h1;
    c[0][0] += 64'hFF_0000_0000;
    d[0][0] = 64'hF_FFFF_FFFF;
    d[0][0]++;
  end
  initial begin
    repeat (2) @(posedge clk);
    #1 r1 = c[0][0]; r2 = d[0][0];
    $finish;
  end
endmodule"#,
        &[("r1", 0xFF_0000_0001), ("r2", 0x10_0000_0000)],
    );
}

#[test]
fn in_a_generate_block_and_a_child_instance() {
    expect(
        r#"module child(input logic clk, input logic [63:0] v, output logic [63:0] q);
  logic [63:0] a [1:0][1:0];
  int i = 1, j = 1;
  always_ff @(posedge clk) a[i][j] <= v;
  assign q = a[1][1];
endmodule
module tb;
  logic clk = 0; always #5 clk = ~clk;
  logic [63:0] v = 64'hFEDC_BA98_7654_3210;
  logic [63:0] r1, r2, q;
  child u(.clk(clk), .v(v), .q(q));
  if (1) begin : g
    logic [63:0] a [1:0][1:0];
    always_ff @(posedge clk) a[0][0] <= v;
  end
  initial begin
    repeat (3) @(posedge clk);
    #1 r1 = q; r2 = g.a[0][0];
    $finish;
  end
endmodule"#,
        &[("r1", V), ("r2", V)],
    );
}

// The element width now equals the store width, which admits the store to the
// two-state tier. That tier took the out-of-range arm of the index computation
// for the index itself and dropped the store, once another store kept the block
// on the tier and an interpreted statement kept its index computation unfused.
// A 32-bit element, whose width was already equal, lost its store the same way.

#[test]
fn variable_index_beside_an_interpreted_statement_and_another_store() {
    expect(
        r#"module tb;
  logic clk = 0; always #5 clk = ~clk;
  logic [7:0] s8 = 8'hA5;
  logic [31:0] s32 = 32'hDEAD_BEEF;
  logic [63:0] s64 = 64'hFEDC_BA98_7654_3210;
  logic [7:0] m8 [2][3];
  logic [31:0] m32 [2][3];
  logic [63:0] m64 [2][3];
  logic [3:0][7:0] p [2];
  logic [31:0] q [2];
  logic [1:0] i = 1;
  logic j = 0;
  logic v = 0, w = 0, x = 0;
  logic [7:0] r8;
  logic [31:0] r32;
  logic [63:0] r64;
  always @(posedge clk) begin m8[j][i] <= s8; p[1][2] <= s8; v <= 1; end
  always @(posedge clk) begin m32[j][i] <= s32; p[0][1] <= s8; w <= 1; end
  always @(posedge clk) begin m64[j][i] <= s64; p[0][0] <= s8; x <= 1; end
  initial begin
    repeat (2) @(posedge clk);
    #1 r8 = m8[0][1]; r32 = m32[0][1]; r64 = m64[0][1];
    $finish;
  end
endmodule"#,
        &[("r8", 0xA5), ("r32", 0xDEAD_BEEF), ("r64", V)],
    );
}

#[test]
fn the_interpreted_statement_may_be_a_system_task() {
    expect(
        r#"module tb;
  logic clk = 0; always #5 clk = ~clk;
  logic [31:0] s = 32'hDEAD_BEEF;
  logic [31:0] m [2][3];
  logic [1:0] i = 2;
  logic j = 1;
  logic v = 0;
  logic [31:0] r;
  always @(posedge clk) begin m[j][i] <= s; $display("fired"); v <= 1; end
  initial begin repeat (2) @(posedge clk); #1 r = m[1][2]; $finish; end
endmodule"#,
        &[("r", 0xDEAD_BEEF)],
    );
}

#[test]
fn an_out_of_range_index_still_writes_nothing() {
    // §7.4.6: the guarded arm is the only one that drops the write.
    expect(
        r#"module tb;
  logic clk = 0; always #5 clk = ~clk;
  logic [31:0] s = 32'hDEAD_BEEF;
  logic [31:0] m [2][3];
  logic [3:0][7:0] p [2];
  logic [1:0] i = 3;
  logic j = 1;
  logic v = 0;
  logic [31:0] r00, r02, r12;
  initial foreach (m[a, b]) m[a][b] = 32'h5;
  always @(posedge clk) begin m[j][i] <= s; p[1][2] <= 8'h1; v <= 1; end
  initial begin
    repeat (2) @(posedge clk);
    #1 r00 = m[0][0]; r02 = m[0][2]; r12 = m[1][2];
    $finish;
  end
endmodule"#,
        &[("r00", 5), ("r02", 5), ("r12", 5)],
    );
}

// The element width now equals the store width, which admits the store to the
// two-state tier. That tier took the out-of-range arm of the index computation
// for the index itself and dropped the store, once another store kept the block
// on the tier and an interpreted statement kept its index computation unfused.
// A 32-bit element, whose width was already equal, lost its store the same way.

#[test]
fn variable_index_beside_an_interpreted_statement_and_another_store() {
    expect(
        r#"module tb;
  logic clk = 0; always #5 clk = ~clk;
  logic [7:0] s8 = 8'hA5;
  logic [31:0] s32 = 32'hDEAD_BEEF;
  logic [63:0] s64 = 64'hFEDC_BA98_7654_3210;
  logic [7:0] m8 [2][3];
  logic [31:0] m32 [2][3];
  logic [63:0] m64 [2][3];
  logic [3:0][7:0] p [2];
  logic [31:0] q [2];
  logic [1:0] i = 1;
  logic j = 0;
  logic v = 0, w = 0, x = 0;
  logic [7:0] r8;
  logic [31:0] r32;
  logic [63:0] r64;
  always @(posedge clk) begin m8[j][i] <= s8; p[1][2] <= s8; v <= 1; end
  always @(posedge clk) begin m32[j][i] <= s32; p[0][1] <= s8; w <= 1; end
  always @(posedge clk) begin m64[j][i] <= s64; p[0][0] <= s8; x <= 1; end
  initial begin
    repeat (2) @(posedge clk);
    #1 r8 = m8[0][1]; r32 = m32[0][1]; r64 = m64[0][1];
    $finish;
  end
endmodule"#,
        &[("r8", 0xA5), ("r32", 0xDEAD_BEEF), ("r64", V)],
    );
}

#[test]
fn the_interpreted_statement_may_be_a_system_task() {
    expect(
        r#"module tb;
  logic clk = 0; always #5 clk = ~clk;
  logic [31:0] s = 32'hDEAD_BEEF;
  logic [31:0] m [2][3];
  logic [1:0] i = 2;
  logic j = 1;
  logic v = 0;
  logic [31:0] r;
  always @(posedge clk) begin m[j][i] <= s; $display("fired"); v <= 1; end
  initial begin repeat (2) @(posedge clk); #1 r = m[1][2]; $finish; end
endmodule"#,
        &[("r", 0xDEAD_BEEF)],
    );
}

#[test]
fn an_out_of_range_index_still_writes_nothing() {
    // §7.4.6: the guarded arm is the only one that drops the write.
    expect(
        r#"module tb;
  logic clk = 0; always #5 clk = ~clk;
  logic [31:0] s = 32'hDEAD_BEEF;
  logic [31:0] m [2][3];
  logic [3:0][7:0] p [2];
  logic [1:0] i = 3;
  logic j = 1;
  logic v = 0;
  logic [31:0] r00, r02, r12;
  initial foreach (m[a, b]) m[a][b] = 32'h5;
  always @(posedge clk) begin m[j][i] <= s; p[1][2] <= 8'h1; v <= 1; end
  initial begin
    repeat (2) @(posedge clk);
    #1 r00 = m[0][0]; r02 = m[0][2]; r12 = m[1][2];
    $finish;
  end
endmodule"#,
        &[("r00", 5), ("r02", 5), ("r12", 5)],
    );
}

// Forms that were right and have to stay right.

#[test]
fn a_narrow_element_still_keeps_only_its_low_bits() {
    expect(
        r#"module tb;
  logic clk = 0; always #5 clk = ~clk;
  logic [63:0] v = 64'hFEDC_BA98_7654_3210;
  logic [7:0] a [1:0][1:0];
  logic [7:0] r;
  always_ff @(posedge clk) a[0][0] <= v;
  initial begin repeat (2) @(posedge clk); #1 r = a[0][0]; $finish; end
endmodule"#,
        &[("r", 0x10)],
    );
}

#[test]
fn bit_and_part_selects_of_an_element() {
    expect(
        r#"module tb;
  logic clk = 0; always #5 clk = ~clk;
  logic [63:0] b [1:0][1:0] = '{default: 64'h0};
  logic [63:0] p [1:0][1:0] = '{default: 64'h0};
  logic [63:0] r1, r2;
  always_ff @(posedge clk) begin b[0][0][40] <= 1'b1; p[0][0][47:40] <= 8'hAB; end
  initial begin
    repeat (2) @(posedge clk);
    #1 r1 = b[0][0]; r2 = p[0][0];
    $finish;
  end
endmodule"#,
        &[("r1", 0x100_0000_0000), ("r2", 0xAB00_0000_0000)],
    );
}

#[test]
fn an_element_with_packed_dimensions() {
    // The whole element is 80 bits; a store to one packed element of it is 40.
    expect(
        r#"module tb;
  logic clk = 0; always #5 clk = ~clk;
  logic [1:0][39:0] whole [1:0][1:0];
  logic [1:0][39:0] part [1:0][1:0];
  logic [79:0] e1, e2, e3;
  logic [39:0] hi1, lo1, hi2, hi3;
  int i = 1;
  initial begin part[0][0] = 80'h0; part[0][1] = 80'h0; end
  always_ff @(posedge clk) begin
    whole[0][0] <= {40'hEF_0000_0003, 40'h11_0000_0004};
    part[0][0][1] <= 40'hAB_0000_0001;
    part[0][1][i] <= 40'hCD_0000_0002;
  end
  initial begin
    repeat (2) @(posedge clk);
    #1 e1 = whole[0][0]; e2 = part[0][0]; e3 = part[0][1];
    hi1 = e1[79:40]; lo1 = e1[39:0]; hi2 = e2[79:40]; hi3 = e3[79:40];
    $finish;
  end
endmodule"#,
        &[
            ("hi1", 0xEF_0000_0003),
            ("lo1", 0x11_0000_0004),
            ("hi2", 0xAB_0000_0001),
            ("hi3", 0xCD_0000_0002),
        ],
    );
}

#[test]
fn one_dimensional_array_beside_a_two_dimensional_one() {
    expect(
        r#"module tb;
  logic clk = 0; always #5 clk = ~clk;
  logic [63:0] v = 64'hFEDC_BA98_7654_3210;
  logic [63:0] one [1:0];
  logic [63:0] two [1:0][1:0];
  logic [63:0] r1, r2;
  always_ff @(posedge clk) begin one[0] <= v; two[0][0] <= v; end
  initial begin
    repeat (2) @(posedge clk);
    #1 r1 = one[0]; r2 = two[0][0];
    $finish;
  end
endmodule"#,
        &[("r1", V), ("r2", V)],
    );
}

#[test]
fn string_and_real_elements() {
    // A width-0 string element is self-determined; a real one is not resized.
    let sim = simulate(
        r#"module tb;
  logic clk = 0; always #5 clk = ~clk;
  string s [1:0][1:0];
  real x [1:0][1:0];
  int len, frac;
  always @(posedge clk) begin s[0][0] = "a long string"; x[0][0] = 2.5; end
  initial begin
    repeat (2) @(posedge clk);
    #1 len = s[0][0].len(); frac = int'(x[0][0] * 2.0);
    $finish;
  end
endmodule"#,
        1000,
    )
    .expect("simulate failed");
    assert_eq!(sim.get_signal("len").and_then(|v| v.to_u64()), Some(13));
    assert_eq!(sim.get_signal("frac").and_then(|v| v.to_u64()), Some(5));
}
