//! §7.4.3, §11.5.1: a part-select written after an index into a
//! multi-dimensional packed array picks whole ELEMENTS of the next declared
//! dimension. On `logic [3:0][5:0][7:0] a`, `a[0][5:0]` is six 8-bit
//! elements, 48 bits; its RHS is evaluated in that 48-bit context (§11.6.1)
//! and the write covers all of them (§10.3 for the continuous form).
//!
//! The bytecode compiler sized such a target by its label count and placed
//! its labels as bit offsets, so `assign a[0][5:0] = b` wrote bits 5..0 of
//! `a[0]` and left the other 42 bits x. Reads, the always_comb and NBA forms,
//! indexed part-selects and variable indices shared the same arithmetic. The
//! VeeR EH2 interrupt controller seeds its priority tree with
//! `assign level_intpend_id[0][pt.PIC_TOTAL_INT_PLUS1+2:0] = {...}` on
//! `logic [N:0][PIC_TOTAL_INT_PLUS1+2:0][7:0]`; the tree went x at reset
//! release and instruction fetch stalled. Expected values follow the LRM.
//!
//! §11.5.1: such a part-select with a run-time base writes only its elements
//! inside the dimension. The compiled store places the whole path as one bit
//! range in the root signal, which is clipped to the root only, so
//! `a[0][k +: 2]` with `k = 5` wrote its second byte into `a[1][0]`.
//! Verilator matches every line except the four writes past an end, which
//! it lets spill into the neighbouring element as xezim did; §11.5.1 lets a
//! partly out-of-range write affect only the bits in range.

fn lines(src: &str) -> Vec<String> {
    let sim = xezim::simulate(src, 100).expect("simulate");
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| m.starts_with("PASS") || m.starts_with("FAIL"))
        .collect()
}

const SRC: &str = r#"
package cfg_pkg;
  typedef struct packed { int unsigned N; int unsigned M; } cfg_t;
endpackage

module tb;
  import cfg_pkg::*;
  localparam cfg_t pt = '{N: 3, M: 2};
  logic clk = 0;
  logic [2:0] j = 3'd2;
  logic [1:0] sel = 2'd2;
  logic [7:0] x8 = 8'hff, y8 = 8'h01;

  logic [5:0][7:0] b;
  for (genvar i = 0; i < 6; i++) begin : G
    assign b[i] = 8'(i + 1);
  end

  // Continuous assignment to a slice of the middle dimension.
  logic [3:0][5:0][7:0] a1;
  assign a1[0][5:0] = b;
  // The VeeR EH2 shape: bounds from a struct parameter, inside a generate.
  logic [3:0][pt.N+3:0][7:0] lvl;
  if (1) begin : GEN
    assign lvl[0][pt.N+2:0] = {8'h0, 8'h0, 8'h0, b[2:0]};
  end
  // Indexed part-selects, constant base.
  logic [3:0][5:0][7:0] a2;
  assign a2[0][1 +: 3] = 24'h333231;
  assign a2[1][5 -: 2] = 16'h4241;
  // always_comb, variable outer index and variable base.
  logic [3:0][5:0][7:0] a3, a4;
  always_comb for (int i = 0; i < 4; i++) a3[i][2:0] = {8'(i), 8'(i + 16), 8'(i + 32)};
  always_comb begin
    a4 = '0;
    a4[sel][j +: 2] = 16'h8988;
  end
  // NBA, variable outer index and base.
  logic [3:0][5:0][7:0] a5;
  always_ff @(posedge clk) begin
    a5[sel][3:0] <= 32'h19181716;
    a5[1][j -: 2] <= 16'h8a8b;
  end
  // Four dimensions: slices of the second and the third.
  logic [1:0][2:0][3:0][7:0] h;
  assign h[1][2][3:1] = 24'h939291;
  assign h[0][2:1] = 64'ha8a7a6a5a4a3a2a1;
  // A middle dimension that is not zero-based.
  logic [3:0][6:1][7:0] m;
  assign m[0][4:2] = 24'hd3d2d1;
  // An element of an unpacked array of 2-D packed vectors.
  logic [5:0][7:0] u [4];
  assign u[1][3:2] = 16'h1211;
  // §11.6.1: the slice is the RHS context, so the carry is kept.
  logic [3:0][5:0][7:0] c1;
  assign c1[1][4:2] = x8 + y8;
  // Outer-dimension slice through an NBA.
  logic [5:0][7:0] p;
  always_ff @(posedge clk) p[4:3] <= 16'h4443;
  // §11.5.1: run-time bases past an end of their dimension.
  logic [2:0] k = 3'd5, k0 = 3'd0;
  logic [3:0][5:0][7:0] o1, o2, o5;
  always_comb begin
    o1 = '0;
    o1[0][k +: 2] = 16'hbbaa;
  end
  always_comb begin
    o2 = '0;
    o2[1][k0 -: 2] = 16'hccdd;
  end
  always_ff @(posedge clk) begin
    o5 <= '0;
    o5[1][k +: 2] <= 16'h7776;
  end
  logic [15:0] r4;
  always_comb r4 = a1[0][k +: 2];
  // A run too long to store element by element.
  logic [1:0][10000:0][7:0] big;
  logic [13:0] kb = 14'd9998;
  always_comb begin
    big = '0;
    big[0][kb +: 10000] = {10000{8'h5a}};
  end
  // Reads.
  logic [47:0] r1;
  logic [15:0] r2;
  logic [63:0] r3;
  assign r1 = a1[0][5:0];
  always_comb r2 = a1[0][j +: 2];
  always_comb r3 = {a1[0][1:0], a1[0][5:4], a1[0][3:2], 16'h0};

  task automatic chk(string name, logic [255:0] got, logic [255:0] exp);
    if (got === exp) $display("PASS %s", name);
    else $display("FAIL %s got=%h exp=%h", name, got, exp);
  endtask

  initial begin
    #1 clk = 1;
    #1 clk = 0;
    #1;
    chk("assign", a1[0], 48'h060504030201);
    chk("struct param", lvl[0], 56'hxx000000030201);
    chk("indexed up", a2[0], 48'hxxxx333231xx);
    chk("indexed down", a2[1], 48'h4241xxxxxxxx);
    chk("comb loop", {a3[3][2:0], a3[2][2:0], a3[1][2:0], a3[0][2:0]},
        96'h031323_021222_011121_001020);
    chk("comb var", a4, {48'h0, 48'h000089880000, 48'h0, 48'h0});
    chk("nba var outer", a5[2], 48'hxxxx19181716);
    chk("nba var base", a5[1], 48'hxxxxxx8a8bxx);
    chk("4-D inner", h[1][2], 32'h939291xx);
    chk("4-D middle", h[0], 96'ha8a7a6a5a4a3a2a1xxxxxxxx);
    chk("offset dim", m[0], 48'hxxxxd3d2d1xx);
    chk("unpacked elem", u[1], 48'hxxxx1211xxxx);
    chk("context", c1[1][4:2], 24'h000100);
    chk("nba outer", p, 48'hxx4443xxxxxx);
    chk("read", r1, 48'h060504030201);
    chk("read var", r2, 16'h0403);
    chk("read concat", r3, 64'h0201_0605_0403_0000);
    chk("past high end", o1, {48'h0, 48'h0, 48'h0, 48'haa0000000000});
    chk("below low end", o2, {48'h0, 48'h0, 48'h0000000000cc, 48'h0});
    chk("nba past end", o5, {48'h0, 48'h0, 48'h760000000000, 48'h0});
    chk("read past end", r4, 16'hxx06);
    chk("long run past end", {big[1][1:0], big[0][10000:9997]}, 48'h00005a5a5a00);
    $finish;
  end
endmodule
"#;

#[test]
fn part_select_of_middle_packed_dimension_selects_whole_elements() {
    let got = lines(SRC);
    let expected: Vec<String> = [
        "assign",
        "struct param",
        "indexed up",
        "indexed down",
        "comb loop",
        "comb var",
        "nba var outer",
        "nba var base",
        "4-D inner",
        "4-D middle",
        "offset dim",
        "unpacked elem",
        "context",
        "nba outer",
        "read",
        "read var",
        "read concat",
        "past high end",
        "below low end",
        "nba past end",
        "read past end",
        "long run past end",
    ]
    .iter()
    .map(|n| format!("PASS {n}"))
    .collect();
    assert_eq!(got, expected);
}

/// §11.5.1: a signed base narrower than 32 bits keeps its value. With
/// `bit signed [7:0] j = -1`, `a[0][j +: 2]` selects labels -1 and 0, as an
/// `int` base of -1 does, so its high byte lands in `a[0][0]`. The compiled
/// path added the 8-bit pattern 255 to 32-bit constants instead: slot 256,
/// so the store wrote nothing and the read returned all x. An unsigned 255
/// is still label 255. Verilator does not follow §11.5.1 once a select
/// leaves its dimension, whatever the base's type, but it gives the `int`
/// base the same results as the 8-bit one.
const SIGNED_BASE_SRC: &str = r#"
module tb;
  logic clk = 0;
  bit signed [7:0] j = -8'sd1;
  logic signed [3:0] jn = -4'sd1;
  bit [7:0] ju = 8'hff;
  int k = -1;
  bit [1:0][5:0][7:0] a, ai, u;
  always_comb begin
    a = '0;
    a[0][j +: 2] = 16'hbbaa;
  end
  always_comb begin
    ai = '0;
    ai[0][k +: 2] = 16'hbbaa;
  end
  always_comb begin
    u = '0;
    u[0][ju +: 2] = 16'hbbaa;
  end
  logic [1:0][5:0][7:0] n;
  always_ff @(posedge clk) begin
    n <= '0;
    n[1][jn +: 3] <= 24'h332211;
  end
  logic [1:0][5:0][7:0] s;
  initial for (int i = 0; i < 12; i++) s[i / 6][i % 6] = 8'(8'h10 + i);
  logic [15:0] r1, r2, r3;
  always_comb r1 = s[1][j +: 2];
  always_comb r2 = s[1][k +: 2];
  always_comb r3 = s[1][ju +: 2];

  task automatic chk(string name, logic [95:0] got, logic [95:0] exp);
    if (got === exp) $display("PASS %s", name);
    else $display("FAIL %s got=%h exp=%h", name, got, exp);
  endtask

  initial begin
    #1 clk = 1;
    #1 clk = 0;
    #1;
    chk("signed base", a, {48'h0, 48'h0000000000bb});
    chk("int base", ai, {48'h0, 48'h0000000000bb});
    chk("unsigned base", u, 96'h0);
    chk("signed base, nba", n, {48'h000000003322, 48'h0});
    chk("signed base, read", r1, 16'h16xx);
    chk("int base, read", r2, 16'h16xx);
    chk("unsigned base, read", r3, 16'hxxxx);
    $finish;
  end
endmodule
"#;

#[test]
fn narrow_signed_base_of_a_middle_dimension_keeps_its_sign() {
    let got = lines(SIGNED_BASE_SRC);
    let expected: Vec<String> = [
        "signed base",
        "int base",
        "unsigned base",
        "signed base, nba",
        "signed base, read",
        "int base, read",
        "unsigned base, read",
    ]
    .iter()
    .map(|n| format!("PASS {n}"))
    .collect();
    assert_eq!(got, expected);
}
