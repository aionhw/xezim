// §9.4.2.2, §20.3.3: replay retains the invoking module time unit.
// §4.7: only aux has competing writers; stamp is independent of their order.
// Multiple writers are legal for always @*, unlike always_comb (§9.2.2.2.2).
module source_unit(output bit ready);
  timeunit 1fs; timeprecision 1fs;
  initial begin ready = 0; #1ns; ready = 1; top.aux = 0; end
endmodule
module top;
  timeunit 1s; timeprecision 1fs;
  wire ready;
  real stamp;
  int aux;
  source_unit s(ready);
  always @* if (ready) begin stamp = $realtime; aux = 1; end
  initial begin #2ns;
    $display("REPLAY stamp=%g", stamp);
    if (stamp != 1e-9) $fatal;
    $display("PASS"); $finish;
  end
endmodule
