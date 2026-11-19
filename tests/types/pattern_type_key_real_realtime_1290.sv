// §6.12: realtime declarations are treated synonymously with real; §10.9.1
// and §10.9.2: the last matching type key wins, before default:.
module top;
  typedef struct {real r; realtime t; shortreal s; int i;} ST;
  localparam real P[2] = '{real:1.5, realtime:2.5, default:9.0};
  real a[2], c[2];
  realtime b[2];
  ST s1, s2;
  initial begin
    a = '{real:1.5, realtime:2.5, default:7.0};
    b = '{realtime:1.5, real:2.5, default:7.0};
    c = '{real:1.5, default:9.0};
    s1 = '{real:1.5, realtime:2.5, shortreal:3.5, default:7};
    s2 = '{shortreal:3.5, realtime:2.5, default:7};
    $display("T|array %0.1f %0.1f", a[0], a[1]);
    $display("T|realtime array %0.1f %0.1f", b[0], b[1]);
    $display("T|one key %0.1f %0.1f", c[0], c[1]);
    $display("T|struct %0.1f %0.1f %0.1f %0d", s1.r, s1.t, s1.s, s1.i);
    $display("T|struct later %0.1f %0.1f %0.1f %0d", s2.r, s2.t, s2.s, s2.i);
    $display("T|localparam %0.1f %0.1f", P[0], P[1]);
  end
endmodule
