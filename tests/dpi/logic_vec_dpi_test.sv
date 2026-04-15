module logic_vec_dpi_test;
  import "DPI-C" function void vec_in(input logic [95:0] x);
  import "DPI-C" function void vec_flip(inout logic [95:0] x);
  import "DPI-C" function void vec_set(output logic [95:0] x);
  import "DPI-C" function int vec_seen_lsb();

  logic [95:0] v;
  int seen;

  initial begin
    v = 96'h00000000_00000000_AAAAAA55;
    vec_in(v);
    seen = vec_seen_lsb();
    vec_flip(v);
    if (v[31:0] !== 32'hAAAAAAAA) begin
      $display("TEST_FAIL: vec_flip writeback wrong: %h", v[31:0]);
      $finish;
    end

    vec_set(v);
    $display("DPI_VEC v=%h seen=%h", v, seen);
    if ((v[31:0] == 32'hAABBCCDD) &&
        (v[63:32] == 32'h55667788) &&
        (v[95:64] == 32'h11223344)) begin
      $display("TEST_PASS");
    end else begin
      $display("TEST_FAIL");
    end
    $finish;
  end
endmodule
