`timescale 1ns/1ps
module top;
  function automatic int width_count();
    bit a[$rtoi((1'b1 ? 2'b11 : 8'd0) + 1'b1)];
    return $size(a);
  endfunction
  function automatic real half_root();
    real x;
    x = 5;
    return $sqrt(x / 2);
  endfunction
  localparam real P = half_root();
  initial begin
    #1;
    if (width_count() == 4) $display("PASS width"); else $display("FAIL width=%0d expected=4", width_count());
    if (P > 1.58113 && P < 1.58114) $display("PASS P"); else $display("FAIL P=%.6f expected=1.581139", P);
  end
endmodule
