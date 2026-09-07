//! §7.4.6: a store to a 2-D unpacked array element (`a[i][j] <= v`) compiles.
//!
//! The READ path already flattened `a[i][j]` row-major over one dense operand;
//! the store had no arm and bailed (`nba_index_other`), which also dragged any
//! enclosing loop onto the AST path (`For_step_other`). A 4x4 array written
//! element-wise every cycle ran ~24x slower than the 1-D equivalent doing the
//! same number of writes (1.92s vs 0.08s); it is now 0.16s.
//!
//! The store reuses the read's flat index, so the out-of-range guard is shared
//! and an index out of range in either dimension is discarded rather than
//! aliasing onto a valid element — which is the case worth pinning here.
//!
//! Widening the loop gate to admit the shape also required teaching the
//! self-read audit (`lv_base_name`/`lv_base_full`) to peel the outer index,
//! or `m[i][j] <= m[i][j] + 1` would have slipped past the alias guard that
//! keeps such updates on the AST path.
//!
//! Every expected value is the reference simulator's.

use xezim::simulate;

fn out(src: &str) -> String {
    let sim = simulate(src, 200).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn two_d_stores_match_the_reference_across_bound_shapes() {
    let o = out(r#"
module tb;
  logic clk=0; int unsigned cyc=0; integer i,j;
  logic [7:0] m  [0:3][0:3];
  logic [7:0] d  [3:0][3:0];      // descending bounds
  logic [7:0] nz [2:5][1:4];      // non-zero-based
  logic [7:0] sr [0:3][0:3];      // self-read
  logic [7:0] bl [0:3][0:3];      // blocking writes (stay on the AST path)
  logic [3:0] oob = 9;            // out of range
  always #5 clk=~clk;
  initial for (i=0;i<4;i++) for (j=0;j<4;j++) begin
    m[i][j]=0; sr[i][j]=1; bl[i][j]=0; end
  always @(posedge clk) begin
    cyc <= cyc+1;
    for (i=0;i<4;i++) for (j=0;j<4;j++) begin
      m[i][j]  <= cyc[7:0]+i[7:0]*8'd4+j[7:0];
      sr[i][j] <= sr[i][j] + 8'd1;
    end
    d[1][2]  <= cyc[7:0];
    nz[3][2] <= cyc[7:0]+8'd7;
    m[oob][1] <= 8'hFF;                      // must be discarded
    for (i=0;i<4;i++) bl[i][0] = cyc[7:0];
  end
  initial begin repeat(6) @(posedge clk); #1;
    $display("M=%02x %02x SR=%02x D=%02x NZ=%02x BL=%02x M01=%02x",
             m[0][0], m[3][3], sr[2][2], d[1][2], nz[3][2], bl[2][0], m[0][1]);
    $finish; end
endmodule
"#);
    // Reference simulator. M01 is the out-of-range check: writing m[9][1]
    // must not land anywhere, so m[0][1] keeps its loop-written value.
    assert!(
        o.contains("M=05 14 SR=07 D=05 NZ=0c BL=05 M01=06"),
        "2-D stores diverge from the reference:\n{o}"
    );
}

/// The looped form is the one that was slow: the statement bail took the whole
/// loop with it. Guard against a silent return to the AST path.
#[test]
fn a_two_d_store_loop_compiles() {
    let o = out(r#"
module tb;
  logic clk=0; logic [7:0] m [0:15][0:15]; int unsigned cyc=0; integer i,j;
  always #5 clk=~clk;
  always @(posedge clk) begin
    cyc <= cyc+1;
    for (i=0;i<16;i++) for (j=0;j<16;j++) m[i][j] <= cyc[7:0]+i[7:0]+j[7:0];
  end
  initial begin repeat (8) @(posedge clk); #1;
    $display("O=%02x", m[3][4]); $finish; end
endmodule
"#);
    // Reference simulator: O=0e
    assert!(o.contains("O=0e"), "looped 2-D store diverges:\n{o}");
}
