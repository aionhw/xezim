// §6.12: realtime is treated synonymously with real, so a `real:` and a
// `realtime:` key both match a real or realtime element; §10.9.1: the last
// matching type key wins.
module top;
  typedef struct {real a[2]; realtime b[2]; real d[2];} S;
  S v;
  initial begin
    v = '{a:'{real:1.5, realtime:9.0, default:9.0},
          b:'{realtime:1.5, real:9.0, default:9.0},
          d:'{real:1.5, realtime:9.0, 0:3.0, default:9.0}};
    $display("T|a %0.1f %0.1f", v.a[0], v.a[1]);
    $display("T|b %0.1f %0.1f", v.b[0], v.b[1]);
    $display("T|d %0.1f %0.1f", v.d[0], v.d[1]);
  end
endmodule
