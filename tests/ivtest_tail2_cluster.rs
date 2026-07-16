//! Hard-tail ivtest behaviors: representative reductions of the remaining
//! ivtest failures (multi-dim packed selects, package scoping, time literals,
//! ...). Each asserts the self-checking "PASSED" marker.

use xezim::simulate;

fn passes(src: &str) -> bool {
    match simulate(src, 100_000) {
        Ok(sim) => {
            let out: String = sim.output.iter().map(|o| o.message.clone()).collect::<Vec<_>>().join("\n");
            out.contains("PASSED") && !out.contains("FAILED")
        }
        Err(_) => false,
    }
}

/// §11.4.1: an lvalue index with a side effect (`x[i++] = v`,
/// `x[i++] += 2`) evaluates the index exactly once (ivtest pr3390385).
#[test]
fn lvalue_index_side_effect_single_eval() {
    assert!(passes(r#"
module tb;
reg [1:0] i, j;
reg [3:0] x[0:2];
reg error;
initial begin
   error = 0;
   i = 0;
   j = i++;
   if (i !== 2'b01 || j !== 2'b00) error = 1;
   i = 0;
   x[0] = 4'dx; x[1] = 4'dx;
   x[i++] = 0;
   if (x[0] !== 4'd0 || x[1] !== 4'dx || i !== 2'd1) error = 1;
   i = 0;
   x[0] = 1;
   x[i++] += 2;
   if (x[0] !== 4'd3) error = 1;
   if (i !== 2'd1) error = 1;
   if (error == 0) $display("PASSED"); else $display("FAILED");
end
endmodule
"#));
}

/// §7.4.1: nested index on a 3-D packed vector selects the element slice,
/// with $bits agreeing at each level (ivtest br_gh112a).
#[test]
fn packed_3d_nested_index_descending() {
    assert!(passes(r#"
module t;
reg [1:0][15:0][7:0] array;
reg failed = 0;
integer i;
reg [3:0] index;
initial begin
  if ($bits(array) !== 256) failed = 1;
  if ($bits(array[0]) !== 128) failed = 1;
  if ($bits(array[0][0]) !== 8) failed = 1;
  for (i = 0; i < 16; i++) begin
    index = i[3:0];
    array[0][index] = {4'd0, index};
    array[1][index] = {4'd1, index};
  end
  if (array !== 256'h1f1e1d1c1b1a191817161514131211100f0e0d0c0b0a09080706050403020100)
    failed = 1;
  for (i = 0; i < 16; i++) begin
    index = i[3:0];
    if (array[0][index] !== {4'd0, index}) failed = 1;
    if (array[1][index] !== {4'd1, index}) failed = 1;
  end
  if (failed) $display("FAILED"); else $display("PASSED");
end
endmodule
"#));
}

/// §7.4.1: ascending packed ranges label the LEFT bound as the
/// most-significant element (ivtest br_gh112b).
#[test]
fn packed_3d_nested_index_ascending() {
    assert!(passes(r#"
module t;
reg [0:1][0:15][0:7] array;
reg failed = 0;
integer i;
reg [3:0] index;
initial begin
  for (i = 0; i < 16; i++) begin
    index = i[3:0];
    array[0][index] = {4'd0, index};
    array[1][index] = {4'd1, index};
  end
  if (array !== 256'h000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f)
    failed = 1;
  for (i = 0; i < 16; i++) begin
    index = i[3:0];
    if (array[0][index] !== {4'd0, index}) failed = 1;
    if (array[1][index] !== {4'd1, index}) failed = 1;
  end
  if (failed) $display("FAILED"); else $display("PASSED");
end
endmodule
"#));
}

/// §7.4.1: non-zero-based and negative packed bounds normalize per
/// dimension, with signed index expressions (ivtest br_gh112c/e/f).
#[test]
fn packed_3d_nested_index_offset_and_negative_bounds() {
    assert!(passes(r#"
module t;
reg [2:1][16:1][8:1] a;
reg [0:-1][14:-1][6:-1] b;
reg failed = 0;
integer i;
reg [3:0] index;
reg signed [4:0] sindex;
initial begin
  for (i = 0; i < 16; i++) begin
    index = i[3:0];
    a[1][index+16'd1] = {4'd0, index};
    a[2][index+16'd1] = {4'd1, index};
    sindex = i[4:0];
    b[-1][-5'sd1+sindex] = {4'd0, sindex[3:0]};
    b[ 0][-5'sd1+sindex] = {4'd1, sindex[3:0]};
  end
  if (a !== 256'h1f1e1d1c1b1a191817161514131211100f0e0d0c0b0a09080706050403020100)
    failed = 1;
  if (b !== 256'h1f1e1d1c1b1a191817161514131211100f0e0d0c0b0a09080706050403020100)
    failed = 1;
  for (i = 0; i < 16; i++) begin
    index = i[3:0];
    if (a[1][index+16'd1] !== {4'd0, index}) failed = 1;
    if (a[2][index+16'd1] !== {4'd1, index}) failed = 1;
  end
  if (failed) $display("FAILED"); else $display("PASSED");
end
endmodule
"#));
}
