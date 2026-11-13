// IEEE 1800-2023 §5.7.1: literal width/sign, including sized truncation.
// §10.9.2 assigns 7 independently to both members; §10.3, §13.5.1,
// §6.20 and §8.5 supply continuous, argument, constant and property contexts.
// §4.7/§6.8: sample after explicit initialization and settling delays.
`timescale 1ns/1ps
module top;
  typedef struct { int a, b; } S;
  typedef struct packed { byte a, b; } P;
  class Holder; S value; endclass
  function automatic S identity(input S value); return value; endfunction
  S s0, c0, arg0;
  Holder h0;
  localparam P K0 = '{2{8'd7}};
  assign c0 = '{2{7}};
  assign arg0 = identity('{2{7}});
  S s1, c1, arg1;
  Holder h1;
  localparam P K1 = '{2'b10{8'd7}};
  assign c1 = '{2'b10{7}};
  assign arg1 = identity('{2'b10{7}});
  S s2, c2, arg2;
  Holder h2;
  localparam P K2 = '{2'd6{8'd7}};
  assign c2 = '{2'd6{7}};
  assign arg2 = identity('{2'd6{7}});
  S s3, c3, arg3;
  Holder h3;
  localparam P K3 = '{4'sd2{8'd7}};
  assign c3 = '{4'sd2{7}};
  assign arg3 = identity('{4'sd2{7}});
  S s4, c4, arg4;
  Holder h4;
  localparam P K4 = '{66'd2{8'd7}};
  assign c4 = '{66'd2{7}};
  assign arg4 = identity('{66'd2{7}});
  S s5, c5, arg5;
  Holder h5;
  localparam P K5 = '{'h2{8'd7}};
  assign c5 = '{'h2{7}};
  assign arg5 = identity('{'h2{7}});
  initial begin
    h0 = new; s0 = '{1,2}; h0.value = '{1,2};
    h1 = new; s1 = '{1,2}; h1.value = '{1,2};
    h2 = new; s2 = '{1,2}; h2.value = '{1,2};
    h3 = new; s3 = '{1,2}; h3.value = '{1,2};
    h4 = new; s4 = '{1,2}; h4.value = '{1,2};
    h5 = new; s5 = '{1,2}; h5.value = '{1,2};
    #1;
    s0 = '{2{7}}; h0.value = '{2{7}};
    s1 = '{2'b10{7}}; h1.value = '{2'b10{7}};
    s2 = '{2'd6{7}}; h2.value = '{2'd6{7}};
    s3 = '{4'sd2{7}}; h3.value = '{4'sd2{7}};
    s4 = '{66'd2{7}}; h4.value = '{66'd2{7}};
    s5 = '{'h2{7}}; h5.value = '{'h2{7}};
    #1;
    if (s0.a === 7 && s0.b === 7 && c0.a === 7 && c0.b === 7 &&
        arg0.a === 7 && arg0.b === 7 && K0 === 16'h0707 &&
        h0.value.a === 7 && h0.value.b === 7)
      $display("PASS literal 2"); else $display("FAIL literal 2");
    if (s1.a === 7 && s1.b === 7 && c1.a === 7 && c1.b === 7 &&
        arg1.a === 7 && arg1.b === 7 && K1 === 16'h0707 &&
        h1.value.a === 7 && h1.value.b === 7)
      $display("PASS literal 2'b10"); else $display("FAIL literal 2'b10");
    if (s2.a === 7 && s2.b === 7 && c2.a === 7 && c2.b === 7 &&
        arg2.a === 7 && arg2.b === 7 && K2 === 16'h0707 &&
        h2.value.a === 7 && h2.value.b === 7)
      $display("PASS literal 2'd6"); else $display("FAIL literal 2'd6");
    if (s3.a === 7 && s3.b === 7 && c3.a === 7 && c3.b === 7 &&
        arg3.a === 7 && arg3.b === 7 && K3 === 16'h0707 &&
        h3.value.a === 7 && h3.value.b === 7)
      $display("PASS literal 4'sd2"); else $display("FAIL literal 4'sd2");
    if (s4.a === 7 && s4.b === 7 && c4.a === 7 && c4.b === 7 &&
        arg4.a === 7 && arg4.b === 7 && K4 === 16'h0707 &&
        h4.value.a === 7 && h4.value.b === 7)
      $display("PASS literal 66'd2"); else $display("FAIL literal 66'd2");
    if (s5.a === 7 && s5.b === 7 && c5.a === 7 && c5.b === 7 &&
        arg5.a === 7 && arg5.b === 7 && K5 === 16'h0707 &&
        h5.value.a === 7 && h5.value.b === 7)
      $display("PASS literal 'h2"); else $display("FAIL literal 'h2");
    $finish;
  end
endmodule
