//! §11.4.10/§11.6.1 — a BLOCK-LOCAL variable used inside an array INDEX.
//!
//! A `for (int i = ...)` header variable (and any `begin`-block temp) is a
//! compiler REGISTER, not a signal, so `expr_max_width`'s signal lookup missed
//! it and fell back to 0 — "unknown width". An index expression is compiled at
//! `ctx_width = 0` (self-determined), and a shift sizes its left operand as
//! `ctx_width.max(lrm_self_width(left)).max(1)`. Both inputs were 0, so the
//! operand was resized to ONE BIT and the shift masked the result away:
//!
//! ```text
//! arr[(i << 6) + off]   read arr[off] on every iteration
//! ```
//!
//! An AES S-box built as `sbox_mid[i] = sbox[(i << LOG) + in[5:0]]` therefore
//! returned `sbox[in[5:0]]` for every byte — the block ran, it just indexed
//! the wrong element, which is why it looked like the `always_comb` had been
//! optimized away.
//!
//! Only the compiled path was affected; the same loop in an `initial` block is
//! correct, and hoisting the index into a temp (`t = (i<<6)+off; arr[t]`) or
//! writing an explicit cast (`arr[8'(i<<6)]`) both worked around it.

use xezim::simulate;

/// The shift shapes that collapsed, next to the ones that always worked.
const INDEX_SHIFT: &str = r#"
module tb;
  logic [7:0] arr [255:0];
  logic [7:0] shl6[3:0], shl1[3:0], mul[3:0], cast[3:0], tmp[3:0], sum[3:0];
  logic [7:0] off;
  always_comb for (int i = 0; i < 4; ++i) shl6[i] = arr[i << 6];
  always_comb for (int i = 0; i < 4; ++i) shl1[i] = arr[i << 1];
  always_comb for (int i = 0; i < 4; ++i) mul [i] = arr[i * 64];
  always_comb for (int i = 0; i < 4; ++i) cast[i] = arr[8'(i << 6)];
  always_comb for (int i = 0; i < 4; ++i) sum [i] = arr[(i << 6) + off[5:0]];
  always_comb for (int i = 0; i < 4; ++i) begin
    int t; t = (i << 6) + off[5:0]; tmp[i] = arr[t];
  end
  int ok;
  initial begin
    for (int k = 0; k < 256; ++k) arr[k] = k[7:0];
    off = 8'h13;                                  // 19
    #1;
    ok = (shl6[0] ==   0 && shl6[1] ==  64 && shl6[2] == 128 && shl6[3] == 192)
      && (shl1[0] ==   0 && shl1[1] ==   2 && shl1[2] ==   4 && shl1[3] ==   6)
      && (mul [0] ==   0 && mul [1] ==  64 && mul [2] == 128 && mul [3] == 192)
      && (cast[0] ==   0 && cast[1] ==  64 && cast[2] == 128 && cast[3] == 192)
      && (sum [0] ==  19 && sum [1] ==  83 && sum [2] == 147 && sum [3] == 211)
      && (tmp [0] ==  19 && tmp [1] ==  83 && tmp [2] == 147 && tmp [3] == 211);
  end
endmodule
"#;

/// A block-local `logic [7:0]` (not just the loop header's `int`) shifted in an
/// index — the same lookup gap, reached by a different declaration form.
const BLOCK_LOCAL_DECL: &str = r#"
module tb;
  logic [7:0] arr [255:0];
  logic [7:0] o[3:0];
  int ok;
  always_comb begin
    for (int i = 0; i < 4; ++i) begin
      logic [7:0] j;
      j = i[7:0];
      o[i] = arr[j << 6];
    end
  end
  initial begin
    for (int k = 0; k < 256; ++k) arr[k] = k[7:0];
    #1;
    ok = (o[0] == 0 && o[1] == 64 && o[2] == 128 && o[3] == 192);
  end
endmodule
"#;

/// Sibling shapes found by auditing the fix against the pre-fix binary: the
/// same width gap reached through a WRITE index, an indexed part-select, and a
/// ternary. Each is a distinct compile path from the plain read, so each can
/// regress on its own.
///
/// The write case is the worst of the three — `w[(i<<6)] = v` sent all four
/// stores to index 0, so three quarters of the array silently kept its fill
/// value and element 0 held whichever store ran last.
const SIBLING_PATHS: &str = r#"
module tb;
  logic [7:0] arr [255:0];
  logic [7:0] w [255:0];
  logic [63:0] vec;
  logic [3:0] psel [3:0];
  logic [7:0] tern [3:0];
  int ok;
  always_comb begin
    for (int k = 0; k < 256; ++k) w[k] = 8'hEE;
    for (int i = 0; i < 4; ++i) w[i << 6] = i[7:0] + 8'd1;   // WRITE index
  end
  always_comb for (int i = 0; i < 4; ++i) psel[i] = vec[(i << 3) +: 4];
  always_comb for (int i = 0; i < 4; ++i) tern[i] = arr[(i > 0) ? (i << 6) : 8'd7];
  initial begin
    for (int k = 0; k < 256; ++k) arr[k] = k[7:0];
    vec = 64'hDEADBEEF12345678;
    #1;
    ok = (w[0] == 1 && w[64] == 2 && w[128] == 3 && w[192] == 4)
      && (w[5] == 8'hEE)                                  // untouched stays filled
      && (psel[0] == 4'h8 && psel[1] == 4'h6 && psel[2] == 4'h4 && psel[3] == 4'h2)
      && (tern[0] == 7 && tern[1] == 64 && tern[2] == 128 && tern[3] == 192);
  end
endmodule
"#;

/// The reported shape end to end: a two-stage S-box lookup driven entirely by
/// shifted loop variables must return the real table entry.
const SBOX: &str = r#"
module tb;
  localparam int LOG = 6;
  logic [7:0] sbox [255:0];
  logic [7:0] mid [3:0];
  logic [7:0] sel;
  logic [7:0] q;
  always_comb begin
    for (int i = 0; i < 4; ++i) mid[i] = sbox[(i << LOG) + sel[5:0]];
    q = mid[sel[7:6]];
  end
  int ok;
  initial begin
    // A stand-in table with a value that depends on the FULL index, so a
    // truncated index cannot accidentally read the right byte.
    for (int k = 0; k < 256; ++k) sbox[k] = k[7:0] ^ 8'h5A;
    sel = 8'h53; #1;
    ok = (q == (8'h53 ^ 8'h5A));
    sel = 8'hFF; #1;
    ok = ok && (q == (8'hFF ^ 8'h5A));
    sel = 8'h40; #1;
    ok = ok && (q == (8'h40 ^ 8'h5A));
  end
endmodule
"#;

fn ok_flag(src: &str) -> u64 {
    let sim = simulate(src, 1000).expect("simulate failed");
    sim.get_signal("ok")
        .or_else(|| sim.get_signal("tb.ok"))
        .expect("signal 'ok' not found")
        .to_u64()
        .unwrap_or(0)
}

#[test]
fn loop_var_shifted_inside_an_array_index_keeps_its_width() {
    assert_eq!(
        ok_flag(INDEX_SHIFT),
        1,
        "a shift inside an array index truncated its block-local operand"
    );
}

#[test]
fn block_local_decl_shifted_inside_an_array_index_keeps_its_width() {
    assert_eq!(
        ok_flag(BLOCK_LOCAL_DECL),
        1,
        "a block-local declaration lost its width inside an array index"
    );
}

#[test]
fn write_index_partselect_and_ternary_siblings_keep_their_width() {
    assert_eq!(
        ok_flag(SIBLING_PATHS),
        1,
        "a write index, indexed part-select, or ternary lost its block-local width"
    );
}

#[test]
fn two_stage_sbox_lookup_reads_the_right_table_entry() {
    assert_eq!(
        ok_flag(SBOX),
        1,
        "the S-box read the low-6-bit entry instead of the full index"
    );
}
