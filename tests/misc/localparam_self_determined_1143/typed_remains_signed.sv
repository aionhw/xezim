// A TYPED `localparam int` stays 32-bit signed (self-determined sizing must
// NOT apply): `4'd0-4'd1` folds to `-1`, extending to 32 bits, per the
// reference simulator. Guard that #1143's fix does not over-apply.
`timescale 1ns/1ps
module top;
  function automatic integer f();
    localparam int X = 4'd0 - 4'd1;
    return X;
  endfunction
  localparam P = f();
  initial begin
    if (P == -1) $display("PASS typed P=%0d", P);
    else $display("FAIL typed P=%0d (expected -1)", P);
    $finish;
  end
endmodule