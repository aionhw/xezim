// IEEE 1800-2023 §6.6.7, §9.2.2.2.2, §20.3.3: a resolved net
// and a plain port both wake a reader in its own module time unit.
package p;
  timeunit 1s; timeprecision 1fs;
  typedef struct { real f; bit v; } rec;
  function automatic rec mk(real f, bit v); rec r; r.f = f; r.v = v; return r; endfunction
  function automatic rec res(input rec d[]); rec r; r = mk(0.0, 0); foreach (d[i]) if (d[i].v) r = d[i]; return r; endfunction
  nettype rec cnet with res;
endpackage
module src(output bit ready);
  timeunit 1fs; timeprecision 1fs;
  initial begin ready = 0; #1ns; ready = 1; end
endmodule
module top;
  timeunit 1s; timeprecision 1fs;
  import p::*;
  wire ready;
  src s(ready);
  rec r;
  cnet n;
  always_comb r = mk(1.0, ready);
  assign n = r;
  real t_net = 0.0, t_wire = 0.0;
  always_comb if (n.v) t_net = $realtime;      // woken through the resolved net
  always_comb if (ready) t_wire = $realtime;   // woken directly by the port
  initial begin
    #2ns;
    $display("t_net=%g t_wire=%g", t_net, t_wire);
    if (t_wire != 1.0e-9) $fatal(1, "control");
    if (t_net != 1.0e-9) $fatal(1, "FAIL: $realtime in the waking module's unit");
    $display("PASS"); $finish;
  end
endmodule
