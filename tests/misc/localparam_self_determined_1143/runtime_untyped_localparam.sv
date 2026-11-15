// IEEE 1800-2023 §6.20.2: an UNTYPED block-scope `localparam` is
// self-determined. A subroutine-local `localparam Q = 4'd0-4'd1` (which the
// runtime hoists to a static cell) must stay 4-bit 15; a `localparam Q = 24`
// must be 24. Regression for #1143 (runtime path).
`timescale 1ns/1ps
module top;
  function automatic int sub() ;
    localparam Q = 4'd0 - 4'd1;
    return Q;
  endfunction
  function automatic int twenty_four() ;
    localparam Q = 24;
    return Q;
  endfunction
  int X, Y;
  initial begin
    X = sub();
    Y = twenty_four();
    if (X == 15) $display("PASS sub X=%0d", X);
     else $display("FAIL sub X=%0d (expected 15)", X);
    if (Y == 24) $display("PASS big Y=%0d", Y);
     else $display("FAIL big Y=%0d (expected 24)", Y);
    $finish;
  end
endmodule