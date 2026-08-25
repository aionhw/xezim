//! §7.4.6 / §11.5.1 — reading an unpacked array at an UNKNOWN index yields x.
//!
//! The bytecode array-load executors converted the index with
//! `to_u64().unwrap_or(0)`. `Value::to_u64` returns `Some(val_bits & !xz_bits)`
//! — it masks x bits to ZERO and NEVER returns `None` — so an x/z index
//! silently read ELEMENT 0 and the read produced real data instead of x.
//!
//! An AES S-box indexed by an uninitialized input therefore returned table
//! entries, and everything downstream resolved to a definite WRONG value:
//! `out = 0xx000xx` where the reference gives `xxxxxxxx`. Nothing else in that
//! chain was at fault — the x-condition ternary merge (§11.4.11) and the
//! or-reduction of x were already correct.
//!
//! Two parts to the fix, both needed: detect the unknown index with
//! `has_xz()` (so the element resolve MISSES), and make the miss arm yield x
//! at the ELEMENT width. A 1-bit x zero-extends into a wider element, which
//! turned `xxxxxxxx` into `0000000x` — right kind of wrong, still wrong.

use xezim::simulate;

/// The reported shape, reduced: a two-stage masked S-box driven by an
/// uninitialized input must produce all-x, while known inputs still read the
/// real table entries.
const SBOX_X_INPUT: &str = r#"
module tb;
  logic [7:0] sbox [255:0];
  logic [7:0] mid [15:0];
  logic [7:0] masked [15:0];
  logic [15:0] or_trees [7:0];
  logic [7:0] in;                       // never driven -> x
  logic [7:0] out;
  int ok;
  always_comb begin
    for (int idx = 0; idx < 16; ++idx) begin
      mid[idx]    = sbox[(idx << 4) + in[3:0]];
      masked[idx] = (in[7:4] == idx) ? mid[idx] : 8'h00;
    end
  end
  always_comb
    for (int b = 0; b < 8; ++b)
      for (int m = 0; m < 16; ++m) or_trees[b][m] = masked[m][b];
  always_comb
    for (int idx = 0; idx < 8; ++idx) out[idx] = |or_trees[idx];
  initial begin
    for (int k = 0; k < 256; ++k) sbox[k] = k[7:0] ^ 8'h5A;
    #1;
    ok = (out === 8'bxxxxxxxx);
  end
endmodule
"#;

/// The primitive on its own, plus the neighbours that were already correct —
/// so a future change cannot "fix" this by making everything x.
const X_INDEX_PRIMITIVE: &str = r#"
module tb;
  logic [7:0] arr [255:0];
  logic [7:0] xin;                      // never driven -> x
  logic [7:0] r_x, r_known, r_tern;
  logic       r_cmp, r_or;
  int ok;
  always_comb r_x     = arr[xin];       // unknown index -> all x
  always_comb r_known = arr[8'd7];      // known index still reads the element
  always_comb r_cmp   = (xin[7:4] == 4'd0);
  always_comb r_tern  = r_cmp ? 8'hAA : 8'h00;
  always_comb r_or    = |xin;
  initial begin
    for (int k = 0; k < 256; ++k) arr[k] = k[7:0];
    #1;
    ok = (r_x === 8'bxxxxxxxx)          // was 8'h00 (element 0)
      && (r_known === 8'd7)
      && (r_cmp === 1'bx)
      && (r_tern === 8'bx0x0x0x0)       // §11.4.11 per-bit merge
      && (r_or === 1'bx);
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
fn masked_sbox_with_unknown_input_stays_x() {
    assert_eq!(ok_flag(SBOX_X_INPUT), 1, "an x input did not propagate through the S-box");
}

#[test]
fn unknown_array_index_reads_x_at_element_width() {
    assert_eq!(ok_flag(X_INDEX_PRIMITIVE), 1, "an x-indexed array read did not yield element-width x");
}
