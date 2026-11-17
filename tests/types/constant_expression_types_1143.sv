// IEEE 1800-2023: constant-expression values the typed constant evaluator
// keeps (#1143). Each line names its case number in the issue.
package p1143;
  bit [2:0] a = 3'sb110 ^ 3'sb001;
endpackage
class C1143;
  localparam int P = 4'sd3 - (1 ? 4'sd5 : 4'd0);
endclass
module child_inside #(parameter P = 0 ? (1 inside {1}) : 16'h8000);
  initial #1 $display("7 inside_arm_bits %0d", $bits(P));
endmodule
module top;
  localparam P1 = (~2'sb10) + $clog2(8);
  localparam P2 = 2'b1x ? 3'd7 : 3'd0;
  localparam N8 = 1 ? (3'sd3 * -3'sd2) : 4'sd0;
  localparam P8 = $countones((N8 < 0) - $isunknown(1'bx));
  localparam P9 = 65'shffffffffffffffff - (8'h1 ^ (~4'h7));
  real r9 = P9;
  localparam P11 = 1 ? (3'sd3 * 3'sd3 * 3'sd2) : (8'd2 ** 1.0);
  function automatic logic signed [3:0] f12(); return -4'sd7; endfunction
  localparam P12 = f12() ** 2'd3;
  localparam Q12 = {P12, 2'b01};
  function automatic int f14();
    case (4'sd3 - (1 ? 4'sd5 : 4'd0))
      -2: return 1;
      default: return 2;
    endcase
  endfunction
  localparam P14 = f14();
  int w6;
  if (1) begin : g
    localparam bit P4 = 1'bx ? 1 : 0;
    initial #2 $display("4 two_state_generate %b", P4);
  end
  child_inside ci();
  initial begin
    #3;
    w6 = p1143::a;
    $display("1 integer_system_result %0d", P1);
    $display("2 known_one_condition %b", P2);
    $display("3 class_int_localparam %0d", C1143::P);
    $display("6 unsigned_package_variable %0d", w6);
    $display("8 isunknown %0d", P8);
    $display("9 wide_to_real %g", r9);
    $display("11 real_exponent_arm %g", P11);
    $display("12 call_width %0d %0d", Q12, $bits(P12));
    $display("14 case_width %0d", P14);
    $finish;
  end
endmodule
