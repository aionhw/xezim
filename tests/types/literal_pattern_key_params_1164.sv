// IEEE 1800-2023 §10.9.1, §11.6.1, §11.8.1, §6.20.2: an array pattern key
// is self-determined, and a parameter operand of the key has its declared
// width and sign: `F + 4'd1` over a 4-bit F of 15 wraps to 0, and
// `S7 + 4'sd1` over a 4-bit signed S7 of 7 is -8.
// §4.7: sample one tick after changing the continuous-assignment source.
package kp;
  localparam bit [3:0] PF = 15;
endpackage
module top;
  localparam bit [3:0] F = 15;
  localparam logic signed [3:0] S7 = 7;
  typedef struct { int a[-16:16]; } S;
  int x;
  localparam int LA[-16:16] = '{(F + 4'd1):7, default:0};
  localparam int LB[-16:16] = '{(S7 + 4'sd1):7, default:0};
  localparam logic [15:0] LK = '{(F + 4'd1):1'b1, default:1'b0};
  int a[-16:16], b[-16:16], c[-16:16], d[-16:16], e[-16:16];
  S s;
  logic [15:0] pk;
  assign a = '{(F + 4'd1):x, default:0};
  assign b = '{(S7 + 4'sd1):x, default:0};
  assign c = '{(~F):x, default:0};
  assign d = '{(kp::PF + 4'd1):x, default:0};
  assign e = '{($signed(F) + 0):x, default:0};
  assign s = '{a:'{(F + 4'd1):x, default:0}};
  assign pk = '{(F + 4'd1):1'b1, default:1'b0};
  initial begin
    #1 x = 7;
    #1;
    $display("localparam %0d %0d %0d %0d %b", LA[0], LA[16], LB[-8], LB[8], LK);
    $display("continuous %0d %0d %0d %0d %0d %0d", a[0], a[16], b[-8], b[8], c[0], c[15]);
    $display("package %0d %0d", d[0], d[16]);
    $display("signed_cast %0d %0d", e[-1], e[15]);
    $display("member %0d %0d", s.a[0], s.a[16]);
    $display("packed %b", pk);
    $finish;
  end
endmodule
