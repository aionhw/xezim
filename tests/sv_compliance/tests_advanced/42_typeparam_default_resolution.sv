// §6.20.3: `parameter type T = expr` — a type-parameter default that
// references a numeric parameter of the same module must resolve against the
// overridden value of that parameter when a sub-instance is inlined.
//
// Without the fix, $bits(val) stays 4 (the module's own default W=4) instead
// of 8 (the caller's override), because the non-overridden T default is never
// re-evaluated against the sub-instance's merged param map.
// Expected output: TEST_PASS
module sub #(parameter int unsigned W = 4, parameter type T = logic[W-1:0]) ();
  T val;
  initial begin
    val = '1;
    #1;
    if ($bits(val) == 8) $display("TEST_PASS typeparam default W=%0d bits=%0d", W, $bits(val));
    else                 $display("TEST_FAIL typeparam default W=%0d bits=%0d (expected 8)", W, $bits(val));
  end
endmodule

module top;
  sub #(.W(8)) u_sub ();
endmodule
