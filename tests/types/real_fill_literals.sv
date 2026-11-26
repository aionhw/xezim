// IEEE 1800-2023 §5.7.1, §11.8.2: real conversion uses a fill's one bit.
package pk;
  parameter real PTWO = 2.0;
endpackage

module top;
  timeunit 1ns; timeprecision 1ps;
  parameter real P = 2.0 + '1;
  real big, one, two, assigned, tiny;
  real ra[2], comb_add, clock_add;
  bit clk;
  shortreal sr;
  realtime rt;
  logic [63:0] all_ones;
  wire eq_one, eq_big, reverse_one, reverse_big;

  assign eq_one = one == '1;
  assign eq_big = big == '1;
  assign reverse_one = '1 == one;
  assign reverse_big = '1 == big;

  // §9.2.2.2, §9.4.2: evaluate in combinational and edge-triggered processes.
  always_comb comb_add = two + '1;
  always @(posedge clk) clock_add = two + '1;

  function automatic real f(real x);
    return x;
  endfunction

  initial begin
    big = 18446744073709551615.0;
    one = 1.0; two = 2.0; sr = 1.0; rt = 1.0; all_ones = '1;
    assigned = '1;
    ra[0] = 2.0;
    #1;
    clk = 1;
    #1;
    // §11.4.5: convert the other operand to real in either operand order.
    $display("T|equal %b %b %b %b", big == '1, one == '1, '1 == big, '1 == one);
    $display("T|unequal %b %b %b %b", big != '1, one != '1, '1 != big, '1 != one);
    $display("T|continuous %b %b %b %b", eq_big, eq_one, reverse_big, reverse_one);
    $display("T|types %b %b %b %b", sr == '1, '1 == sr, rt == '1, '1 == rt);
    // §6.12.1: use the assignment conversion of X and Z to zero here.
    $display("T|zero %b %b %b %b", one == '0, 0.0 == '0, 0.0 == 'x, 0.0 == 'z);
    $display("T|arithmetic %.1f %.1f %.1f %.1f", two + '1, two * '1, two - '0, assigned);
    // §11.3.1, §11.8.2: real arithmetic converts the self-determined fill.
    $display("T|function %.1f", f(two) + '1);
    $display("T|element %g", ra[0] / '1);
    $display("T|comb %.1f", comb_add);
    $display("T|clocked %.1f", clock_add);
    // §6.20, §26.3: real parameters in module and package scope.
    $display("T|package %.1f", pk::PTWO + '1);
    $display("T|parameter %.1f", P);
    // §11.4.4: known relational operands use the same real conversion.
    $display("T|relations %b %b %b %b", '1 < two, '1 <= one, two > '1, one >= '1);
    // §11.4.13: 2.0 is outside the inclusive real range from 0.0 to 1.0.
    $display("T|range %b", two inside {['0:'1]});
    // §11.6: integral/integral fills still extend to the integral width.
    $display("T|integral %b %b %h %h", all_ones == '1, '1 == all_ones, all_ones + '1, all_ones * '1);
    // §12.5, §20.5: the smallest positive real must not match the value 1.
    tiny = $bitstoreal(64'd1);
    case (tiny)
      '1: $display("T|case 1");
      default: $display("T|case 0");
    endcase
    $finish;
  end
endmodule
