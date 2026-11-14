parameter real R = 8.0;
module top;
  parameter real R = 2.0;
  localparam real P = $ln(((2'b11 + 1'b1) + 8'd0) / 8'd4);
  localparam real U = $ln($unit::R);
  initial #1 begin
    if (P == 0.0) $display("PASS P=%g", P); else $display("FAIL P=%g expected=0", P);
    if (U > 2.0794 && U < 2.0795) $display("PASS U=%g", U); else $display("FAIL U=%g expected=2.07944", U);
  end
endmodule
