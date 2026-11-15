// IEEE 1800-2023 §6.20.2: an UNTYPED block-scope `localparam`/`parameter`
// is self-determined from its initializer. Inside a constant function,
// `localparam Q = 4'd0-4'd1` is a 4-bit value (15), NOT the 32-bit signed
// integer that folds `0-1` to `-1`. Regression for #1143 (const path).
`timescale 1ns/1ps
module top;
  function automatic integer f();
    localparam Q = 4'd0 - 4'd1;
    return Q;
  endfunction
  localparam P = f();
  initial begin
    if (P == 15) $display("PASS const P=%0d", P);
    else $display("FAIL const P=%0d (expected 15)", P);
    $finish;
  end
endmodule