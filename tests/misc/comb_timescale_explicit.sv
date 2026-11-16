// §3.14.2.3, §20.3.1, §20.3.3, §20.4.2: explicit module units and $printtimescale.
module top;
  timeunit 1ns; timeprecision 1ns;
  wire ready;
  source_unit s(ready);
  real stamp;
  int marker = 7;
  always_comb if (ready) begin
    stamp = $realtime;
    $display("DEFAULT scope=%m marker=%0d stamp=%g time=%0d", marker, stamp, $time);
    $printtimescale;
  end
  initial begin #2ns; $printtimescale; $finish; end
endmodule
module source_unit(output bit ready);
  timeunit 1ps; timeprecision 1ps;
  int marker = 99;
  initial begin ready = 0; #1ns; ready = 1; end
endmodule
