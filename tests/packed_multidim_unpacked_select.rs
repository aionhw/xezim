//! §7.4 — indexing an unpacked-array element that itself has PACKED
//! dimensions. For `logic [0:0][31:0] arr [0:0]`, `arr[i]` selects the
//! unpacked element (a 32-bit `[0:0][31:0]` value) and only the SECOND index
//! `arr[i][j]` selects the packed `[0:0]` dimension (a 32-bit slice).
//!
//! xezim's `packed_nested_select` consumed BOTH indices as packed dimensions,
//! so `arr[0][0]` collapsed to a 1-bit select (bit 0) instead of the full
//! 32-bit word. Fixed by making it unpacked-aware: the leading `num_unpacked`
//! indices name the per-element signal, the rest index the packed dims.

use xezim::simulate;

fn line(src: &str) -> Vec<String> {
    simulate(src, 100)
        .expect("sim")
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect()
}

#[test]
fn zero_zero_dims_1d_to_6d() {
    let src = r#"
module tb;
  int fails = 0;
  logic [31:0] arr_1d [0:0];
  logic [0:0][31:0] arr_2d [0:0];
  logic [0:0][31:0] arr_3d [0:0][0:0];
  logic [0:0][0:0][31:0] arr_4d [0:0][0:0];
  logic [0:0][0:0][31:0] arr_5d [0:0][0:0][0:0];
  logic [0:0][0:0][0:0][31:0] arr_6d [0:0][0:0][0:0];
  initial begin
    arr_1d[0] = 32'h1111_1111;
    if (arr_1d[0] !== 32'h1111_1111) fails++;
    arr_2d[0][0] = 32'h2222_2222;
    if (arr_2d[0][0] !== 32'h2222_2222) fails++;
    arr_3d[0][0][0] = 32'h3333_3333;
    if (arr_3d[0][0][0] !== 32'h3333_3333) fails++;
    arr_4d[0][0][0][0] = 32'h4444_4444;
    if (arr_4d[0][0][0][0] !== 32'h4444_4444) fails++;
    arr_5d[0][0][0][0][0] = 32'h5555_5555;
    if (arr_5d[0][0][0][0][0] !== 32'h5555_5555) fails++;
    arr_6d[0][0][0][0][0][0] = 32'h6666_6666;
    if (arr_6d[0][0][0][0][0][0] !== 32'h6666_6666) fails++;
    // whole-element (packed) slice read
    begin
      logic [0:0][0:0][0:0][31:0] slice;
      slice = arr_6d[0][0][0];
      if (slice !== 32'h6666_6666) fails++;
    end
    $display("FAILS=%0d", fails);
  end
endmodule
"#;
    let out = line(src);
    assert!(
        out.iter().any(|m| m == "FAILS=0"),
        "packed-multidim + unpacked select wrong; got {:?}",
        out
    );
}

#[test]
fn negative_packed_index() {
    // `logic [-1:-5][31:0] d [-2:-4]`: d[u] unpacked element, d[u][p] packed.
    let src = r#"
module tb;
  int fails = 0;
  logic [-1:-5][31:0] d [-2:-4];
  initial begin
    d[-2][-1] = 32'hAAAA_0001;
    d[-3][-5] = 32'hBBBB_0002;
    if (d[-2][-1] !== 32'hAAAA_0001) fails++;
    if (d[-3][-5] !== 32'hBBBB_0002) fails++;
    $display("FAILS=%0d", fails);
  end
endmodule
"#;
    let out = line(src);
    assert!(
        out.iter().any(|m| m == "FAILS=0"),
        "negative packed index select wrong; got {:?}",
        out
    );
}
