// IEEE 1800-2023 §5.7.1, §11.6.1, §11.8.1-2, §10.9.1:
// array keys are self-determined; sized addition wraps before indexing.
// §4.7: sample one tick after changing the continuous-assignment source.
`timescale 1ns/1ps
module top;
  localparam int L[0:4] = '{(2'sb11 & 4):7, default:0};
  localparam int LC[0:4] = '{4:7, default:0};
  parameter int P[0:4] = '{(2'sb11 & 4):7, default:0};
  parameter int PC[0:4] = '{4:7, default:0};
  localparam logic [4:0] K = '{(2'sb11 & 4):1'b1, default:1'b0};
  localparam logic [4:0] KC = '{4:1'b1, default:1'b0};
  logic [4:0] v = '{(2'sb11 & 4):1'b1, default:1'b0};
  logic [4:0] vc = '{4:1'b1, default:1'b0};
  logic [4:0] w, wc, p, pc;
  int x;
  assign w = '{(2'sb11 & 4):x[0], default:1'b0};
  assign wc = '{4:x[0], default:1'b0};
  int u[-4:4];
  // Preserve the existing int-index conversion for an unsigned 32-bit key.
  assign u = '{((2'sb11 + 32'd0) - 4):x, default:5};
  int a[-16:16], b[-16:16], c[-16:16], d[-16:16], e[-16:16];
  assign a = '{(4'd15 + 4'd1):x, default:0};
  assign b = '{(~4'd0):x, default:0};
  assign c = '{(4'sd7 + 4'sd1):x, default:0};
  assign d = '{(8 / 2'sb11):x, default:0};
  assign e = '{(4'shf % 3):x, default:0};
  int f[-2:2], g[-2:2];
  assign f = '{(64'shffff_ffff_ffff_ffff):x, default:0};
  assign g = '{('shffff_ffff):x, default:0};
  int h[0:1], j[0:1];
  // §11.4.10: oversized shift counts retain the existing key path.
  assign h = '{((~(4'sb1000 >= 32'hffff_ffff)) != ((6'sd21 != 16'shffff) >> (3'sd6 ? 33'sh1_0000_0000 : 4'sb1000))):x, default:0};
  assign j = '{(((7 & 1) <= (((-8'shfe) & (-4'shf)) >= (!(4'b1010 >>> 33'h1_0000_0001)))) & 6'd31):x, default:0};
  initial begin
    #1 x=7;
    p = '{(2'sb11 & 4):1'b1, default:1'b0};
    pc = '{4:1'b1, default:1'b0};
    #1;
    $display("localparam %0d %0d %0d %0d", L[0], L[4], LC[0], LC[4]);
    $display("parameter %0d %0d %0d %0d", P[0], P[4], PC[0], PC[4]);
    $display("packed_constant %b %b", K, KC);
    $display("packed_initializer %b %b", v, vc);
    $display("packed_continuous %b %b", w, wc);
    $display("packed_procedural %b %b", p, pc);
    $display("unsigned_index %0d %0d", u[-1], u[0]);
    $display("width %0d %0d %0d", a[0], b[15], c[-8]);
    $display("divide_remainder %0d %0d", d[-8], e[-1]);
    $display("signed_high_bit %0d %0d", f[-1], g[-1]);
    $display("shift_count %0d %0d %0d %0d", h[0], h[1], j[0], j[1]);
    $finish;
  end
endmodule
