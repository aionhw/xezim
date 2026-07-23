// Compliance test: packed-struct assignment patterns and packed-2D element assign.
//
// Fix 1 (fabe866): named assignment patterns into a packed struct were packed
// by naive reverse-concat, so any unsized literal ('0, 32 bits) shifted every
// field off its correct position.  Fix: pack by member offset via
// packed_struct_fields.
//
// Fix 2 (b33014e): `assign a[i] = b[j]` on `logic [N-1:0][W-1:0]` (elem_w>1)
// was fused as a 1-bit gate because try_resolve_bit_ref resolved the LHS
// element to a single-bit index.  Fix: bail when elem_w > 1 so the full-width
// element continuous assign is emitted instead.
package pkg;
  typedef logic [1:0] x_bits_t;
  typedef logic [1:0] y_bits_t;
  typedef logic [0:0] port_id_t;
  typedef struct packed {
    x_bits_t  x;
    y_bits_t  y;
    port_id_t port_id;
  } xy_id_t;  // 5-bit packed: {x[1:0], y[1:0], port_id[0]}
endpackage

module top;
  import pkg::*;

  // --- Fix 1a: continuous-assign with named pattern ---
  logic [1:0] sx = 2'd2;
  xy_id_t ca_v;
  assign ca_v = '{x: sx, y: 2'd1, port_id: '0};
  // expect: x=10 y=01 port_id=0 => 10_01_0 = 5'b10010

  // --- Fix 1b: NBA with named pattern ---
  xy_id_t nba_v;
  logic   clk = 0;
  always #5 clk = ~clk;
  initial begin
    @(posedge clk);
    nba_v <= '{x: 2'd2, y: 2'd1, port_id: '0};
    @(posedge clk);
  end

  // --- Fix 2: packed-2D element continuous assign ---
  logic [4:0][7:0] src, dst;
  assign dst[2] = src[3];

  initial begin
    src[3] = 8'hAB;
    #1;
    // packed-struct CA check (at t=1 the assign has settled)
    if (ca_v !== 5'b10010) begin
      $display("TEST_FAIL: packed-struct CA: ca_v=%b (expect 10010)", ca_v);
      $finish;
    end
    // packed-2D element assign check
    if (dst[2] !== 8'hAB) begin
      $display("TEST_FAIL: packed-2D elem assign: dst[2]=%h (expect AB)", dst[2]);
      $finish;
    end
    // NBA check requires clock edge; wait for nba_v to be written
    @(posedge clk); @(posedge clk);
    if (nba_v !== 5'b10010) begin
      $display("TEST_FAIL: packed-struct NBA: nba_v=%b (expect 10010)", nba_v);
      $finish;
    end
    $display("TEST_PASS");
    $finish;
  end
endmodule
