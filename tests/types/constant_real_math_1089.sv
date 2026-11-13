`timescale 1ns/1ps
package math_pkg;
  localparam real LP = $ln(8.0);
  localparam real FP = $exp($ln(8.0));
endpackage
module width_child #(parameter logic [7:0] N=8'hff, parameter real R=8.0);
  function automatic int narrow(input logic [$rtoi(N + 8'd1):0] x); return $bits(x); endfunction
  function automatic int math_width(input logic [$rtoi($ln(R)):0] x); return $bits(x); endfunction
endmodule
module top;
  localparam real P_ln = $ln(8.0);
  localparam real P_log10 = $log10(1000.0);
  localparam real P_exp = $exp(1.0);
  localparam real P_sqrt = $sqrt(9.0);
  localparam real P_floor = $floor(-1.2);
  localparam real P_ceil = $ceil(-1.2);
  localparam real P_pow = $pow(2.0,3.0);
  localparam real P_sin = $sin(0.5);
  localparam real P_cos = $cos(0.5);
  localparam real P_tan = $tan(0.5);
  localparam real P_asin = $asin(0.5);
  localparam real P_acos = $acos(0.5);
  localparam real P_atan = $atan(0.5);
  localparam real P_atan2 = $atan2(1.0,-1.0);
  localparam real P_hypot = $hypot(3.0,4.0);
  localparam real P_sinh = $sinh(0.5);
  localparam real P_cosh = $cosh(0.5);
  localparam real P_tanh = $tanh(0.5);
  localparam real P_asinh = $asinh(0.5);
  localparam real P_acosh = $acosh(2.0);
  localparam real P_atanh = $atanh(0.5);
  localparam real BITS = $ln($bitstoreal(64'h3ff0000000000000));
  localparam real ROUNDTRIP = $ln($bitstoreal($realtobits(8.0)));
  localparam real SHORT = $ln($bitstoshortreal(32'h3f800000));
  localparam int W = $rtoi($log10(1000.0));
  logic [W-1:0] data;
  width_child u();
  width_child #(.N(8'hfe), .R(64.0)) v();
  initial begin
    #1;
    $display("T|ln %0.6f", P_ln);
    $display("T|log10 %0.6f", P_log10);
    $display("T|exp %0.6f", P_exp);
    $display("T|sqrt %0.6f", P_sqrt);
    $display("T|floor %0.6f", P_floor);
    $display("T|ceil %0.6f", P_ceil);
    $display("T|pow %0.6f", P_pow);
    $display("T|sin %0.6f", P_sin);
    $display("T|cos %0.6f", P_cos);
    $display("T|tan %0.6f", P_tan);
    $display("T|asin %0.6f", P_asin);
    $display("T|acos %0.6f", P_acos);
    $display("T|atan %0.6f", P_atan);
    $display("T|atan2 %0.6f", P_atan2);
    $display("T|hypot %0.6f", P_hypot);
    $display("T|sinh %0.6f", P_sinh);
    $display("T|cosh %0.6f", P_cosh);
    $display("T|tanh %0.6f", P_tanh);
    $display("T|asinh %0.6f", P_asinh);
    $display("T|acosh %0.6f", P_acosh);
    $display("T|atanh %0.6f", P_atanh);
    $display("T|bits %0.6f %0.6f %0.6f",BITS,ROUNDTRIP,SHORT);
    $display("T|width %0d",$bits(data));
    $display("T|signature %0d %0d %0d %0d",u.narrow('0),v.narrow('0),u.math_width('0),v.math_width('0));
    $display("T|package %0.6f %0.6f",math_pkg::LP,math_pkg::FP);
    $finish;
  end
endmodule
