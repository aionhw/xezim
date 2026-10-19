//! §7.4.1, §11.6.1: a continuous assign or `always_comb` write to an ELEMENT
//! of a multi-dimensional packed signal, `assign d[i][j] = v;`, where the
//! element is wider than 32 bits.
//!
//! `infer_lhs_width` resolved the width of a single select on an identifier,
//! but a NESTED select (`Index(Index(d, i), j)`) fell through to its 32-bit
//! default. `compile_cont_assign_lhs` and the procedural blocking-assign path
//! both size the RHS by that width before the store, so the value was cut to
//! 32 bits — `MoveResize(.., 32)` then `Resize(.., 40)` — while the store
//! itself wrote the correct, full-width element range. The upper bits of
//! every such element read 0. The packed-path store already worked around
//! the same gap by deriving strides from the declared dimensions; the width
//! now comes from those dimensions too: the product of the levels the chain
//! leaves unconsumed.
//!
//! The procedural `initial` write was always right (it never asked
//! `infer_lhs_width`), as was a single select on a 2-D signal. Expected
//! values are the reference simulator's.

use xezim::simulate;

fn out(src: &str) -> String {
    let sim = simulate(src, 200).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// 3-D packed vector, element wider than 32 bits: continuous assign and
/// always_comb writes, plus an element READ and a 1-D write as controls.
#[test]
fn wide_element_of_3d_packed_vector() {
    const SRC: &str = r#"
module top;
  logic [3:0][4:0][39:0] b, d_ca, d_ac;
  logic [39:0] rd, src;
  logic [3:0][79:0] one_d;
  logic [79:0] w;
  assign rd = b[1][2];
  assign d_ca[0][0] = src;
  always_comb d_ac[3][4] = b[1][2];
  assign one_d[1] = w;
  initial begin
    b = '0;
    b[1][2] = 40'hce_0000_0003;
    src = 40'hce_0000_0004;
    w = 80'habcd_face_0000_1234_5678;
    #1;
    $display("rd=%h ca=%h ac=%h one_d=%h", rd, d_ca[0][0], d_ac[3][4], one_d[1]);
  end
endmodule
"#;
    assert_eq!(
        out(SRC),
        "rd=ce00000003 ca=ce00000004 ac=ce00000003 one_d=abcdface000012345678"
    );
}

/// A packed struct element with a unit-width innermost dimension
/// (`flit_t [N-1:0][0:0]`), written per index from a generate loop.
#[test]
fn wide_struct_element_with_unit_inner_dim_from_generate() {
    const SRC: &str = r#"
typedef struct packed { logic [15:0] hdr; logic [63:0] payload; } flit_t;
module top;
  flit_t [4:0] in_flits;
  flit_t [4:0][0:0] data;
  for (genvar p = 0; p < 5; p++) begin : g
    assign data[p][0] = in_flits[p];
  end
  initial begin
    for (int p = 0; p < 5; p++)
      in_flits[p] = '{hdr: 16'hab00 + 16'(p), payload: 64'hface_0000_0000_0000 + 64'(p)};
    #1;
    $display("d0=%h d4=%h", data[0][0], data[4][0]);
  end
endmodule
"#;
    assert_eq!(out(SRC), "d0=ab00face000000000000 d4=ab04face000000000004");
}

/// Element-to-element copies inside an instantiated module, indices from
/// genvars. A partial chain on a 3-D signal
/// (`m[i][j]` on `[3:0][4:0][0:0]` of a struct) still selects one element.
#[test]
fn wide_element_copies_in_submodule_generate() {
    const SRC: &str = r#"
typedef struct packed { logic [15:0] hdr; logic [63:0] payload; } flit_t;
module child;
  flit_t [3:0][4:0][0:0] src, dst;
  for (genvar r = 0; r < 4; r++) begin : gl
    for (genvar d = 0; d < 4; d++) begin : gd
      assign dst[r][d][0] = src[(r + 1) % 4][(d + 2) % 4][0];
    end
  end
  initial begin
    src[1][2][0] = '{hdr: 16'hABCD, payload: 64'hFACE_0000_0000_0003};
    #1;
    $display("dst00=%h dst0_0=%h", dst[0][0][0], dst[0][0]);
  end
endmodule
module top; child u(); endmodule
"#;
    assert_eq!(
        out(SRC),
        "dst00=abcdface000000000003 dst0_0=abcdface000000000003"
    );
}

/// A two-index write on a 3-D signal selects a whole sub-array
/// (`[4:0][39:0]` = 200 bits), not one element.
#[test]
fn partial_chain_write_selects_subarray() {
    const SRC: &str = r#"
module top;
  logic [1:0][2:0][39:0] m;
  logic [2:0][39:0] row;
  assign m[1] = row;
  logic [1:0][2:0][1:0][39:0] n;
  assign n[0][2] = {40'hce_0000_0002, 40'hce_0000_0001};
  initial begin
    row = {40'hce_0000_0012, 40'hce_0000_0011, 40'hce_0000_0010};
    #1;
    $display("m10=%h m12=%h n020=%h n021=%h", m[1][0], m[1][2], n[0][2][0], n[0][2][1]);
  end
endmodule
"#;
    assert_eq!(
        out(SRC),
        "m10=ce00000010 m12=ce00000012 n020=ce00000001 n021=ce00000002"
    );
}
