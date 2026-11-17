// Driver for tests/dpi/vpi_assertion.c — the IEEE 1800-2017 Clause 39
// assertion API: enumeration, attempt callbacks and assertion control.
`timescale 1ns/1ns
module top;
  logic clk = 0;
  logic a = 1, b = 0, c = 0;
  always #5 clk = ~clk;  // posedges at 5, 15, 25, ...

  property p_a;
    @(posedge clk) a;
  endproperty
  A1: assert property (p_a) else $display("A1 fail action at %0t", $time);
  // An attempt that starts on b is in flight for two ticks.
  A2: assert property (@(posedge clk) b |-> ##2 c);
  C1: cover property (@(posedge clk) a);

  initial begin
    $va_setup;
    // A1 passes at 5 and 15, and fails at 25.
    #18 a = 0;
    #10 a = 1;
    // t=28: no A1 attempt at 35 or 45.
    $va_ctl(620, "A1", 0);  // vpiAssertionDisable
    // t=48: A1 attempts again from 55.
    #20 $va_ctl(621, "A1", 0);  // vpiAssertionEnable
    // A2 starts an attempt at 65 that would fail at 85 ...
    #12 b = 1;
    #10 b = 0;
    // ... t=70: killed. A2 stays enabled: its attempt of 85 succeeds at 105.
    $va_ctl(623, "A2", 65);  // vpiAssertionKill
    #10 b = 1;
    #10 b = 0;
    #10 c = 1;
    #10 $va_check;
    $display("RESULT: DONE");
    $finish;
  end
endmodule
