// IEEE 1800-2023 §20.3.1, §20.3.3: time calls use the invoking module.
// §20.4.3, §21.2.1.5 cover formatting and the instance name.
// §9.2.2.2.2 / §9.4.2.2: comb sensitivity follows ready; sample at 3ns,
// away from the 1ns stimulus and the delayed @* update at 2ns (§4.7).
module source_unit(output bit ready);
  timeunit 1fs; timeprecision 1fs;
  initial begin ready = 0; #1ns; ready = 1; end
endmodule
module reader(input wire ready, output real stamp);
  timeunit 1s; timeprecision 1fs;
  always_comb if (ready) stamp = $realtime;
endmodule
module top;
  timeunit 1s; timeprecision 1fs;
  wire ready;
  source_unit s(ready);
  real comb_stamp, star_stamp, edge_stamp, initial_stamp, delayed_stamp;
  real function_stamp, child_stamp;
  time comb_time, star_time;
  reader r(ready, child_stamp);
  function automatic real stamp(); return $realtime; endfunction
  always_comb if (ready) begin
    comb_stamp = $realtime;
    comb_time = $time;
    $display("SCOPE=%m");
    $display("FORMAT time=%0t real=%0t", $time, $realtime);
  end
  always @* if (ready) begin star_stamp = $realtime; star_time = $time; end
  always_comb if (ready) function_stamp = stamp();
  always @(posedge ready) edge_stamp = $realtime;
  always @* if (ready) begin #1ns; delayed_stamp = $realtime; end
  initial begin #1.5ns; initial_stamp = $realtime; end
  initial begin
    #3ns;
    $display("READ comb=%g star=%g edge=%g initial=%g delayed=%g fn=%g child=%g",
      comb_stamp, star_stamp, edge_stamp, initial_stamp, delayed_stamp,
      function_stamp, child_stamp);
    $display("TIME comb=%0d star=%0d", comb_time, star_time);
    if (comb_stamp != 1e-9 || star_stamp != 1e-9 || edge_stamp != 1e-9 ||
        initial_stamp != 1.5e-9 || delayed_stamp != 2e-9 ||
        function_stamp != 1e-9 || child_stamp != 1e-9 || comb_time != 0 || star_time != 0)
      $fatal(1, "wrong invoking module time unit");
    $display("PASS");
    $finish;
  end
endmodule
