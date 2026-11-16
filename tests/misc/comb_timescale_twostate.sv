// IEEE 1800-2023 §20.3.1, §20.3.2. A two-state top-level block keeps
// its arithmetic on the two-state route; only the time call runs through the statement fallback.
// One writer per variable; stimulus at 1ns, sample at 2ns (§4.7).
module source_unit(output bit ready, output bit [7:0] a);
  timeunit 1fs; timeprecision 1fs;
  initial begin ready = 0; a = 8'd3; #1ns; a = 8'd4; ready = 1; end
endmodule
module top;
  timeunit 1us; timeprecision 1fs;
  bit ready; bit [7:0] a, b, c;
  longint t; int unsigned st;
  source_unit s(ready, a);
  always_comb begin b = a + 8'd1; if (ready) t = $time + 64'd100; else t = -1; end
  always @* begin c = a ^ 8'h55; if (ready) st = $stime; else st = 7; end
  initial begin
    #2ns;
    $display("TWOSTATE t=%0d st=%0d b=%0d c=%0d", t, st, b, c);
    if (t != 100 || st != 0 || b != 5 || c != 81) $fatal(1, "wrong invoking module time unit");
    $display("PASS");
    $finish;
  end
endmodule
