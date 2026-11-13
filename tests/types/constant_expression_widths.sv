`timescale 1ns/1ps
module child;
  parameter logic [15:0] HW = 16'h0;
endmodule
module top;
  // Untyped parameters: the context is the whole expression (§11.8.2)
  parameter  P = 1 ? (3'sd3 * 3'sd3 * 3'sd2) : 6'sd0;  // 18
  localparam L = 1 ? (3'sb011 * 3'sb110) : 4'sb0000;   // -6
  localparam Q = (2'b11 + 2'b11) + 3'b000;             // 6
  // Assignment-like contexts (§10.8): the destination supplies the width
  logic [2:0] V = 2'b11 + 2'b11;                       // 6
  function automatic int sq(input logic [2:0] a); return a * a; endfunction
  localparam int C = sq(3'd7);                          // 49
  localparam logic [1:0][15:0] PE = '{(4'd9 - 5'd10) + 8'd0, 16'd0};  // PE[1] = ffff
  child u();
  defparam u.HW = (4'd9 - 5'd10) + 8'd0;               // 00ff: defparam is not assignment-like (§10.8)
  initial begin
    #1 if (P == 18) $display("PASS P=%0d", P); else $display("FAIL P=%0d (expected 18)", P);
       if (L == -6) $display("PASS L=%0d", L); else $display("FAIL L=%0d (expected -6)", L);
       if (Q == 6)  $display("PASS Q=%0d", Q); else $display("FAIL Q=%0d (expected 6)", Q);
       if (V == 6)  $display("PASS V=%0d", V); else $display("FAIL V=%0d (expected 6)", V);
       if (C == 49) $display("PASS C=%0d", C); else $display("FAIL C=%0d (expected 49)", C);
       if (PE[1] == 16'hffff) $display("PASS PE[1]=%h", PE[1]); else $display("FAIL PE[1]=%h (expected ffff)", PE[1]);
       if (u.HW == 16'h00ff) $display("PASS HW=%h", u.HW); else $display("FAIL HW=%h (expected 00ff)", u.HW);
    $finish;
  end
endmodule
