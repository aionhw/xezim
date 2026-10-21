// top: t16_12_8_iff_guard_edge
// LRM §16.12.8: property_expr iff (guard_expr) — edge cases
module t16_12_8_iff_guard_edge;
  bit clk, a, b, guard;
  always #5 clk = ~clk;

  // Test 1: guard always false -> all vacuous passes (no output)
  initial begin
    @(negedge clk); // t=10
    a = 1;
    @(negedge clk); // t=20
    a = 0;
    @(negedge clk); // t=30
    a = 1;
    @(negedge clk); // t=40
    $finish;
  end

  p_always_vacuous: assert property (@(posedge clk) a iff 1'b0)
    $display("T|iff_vacuous|P t=%0t", $time);
  else
    $display("T|iff_vacuous|F t=%0t", $time);

  // Test 2: guard always true -> property must hold every cycle
  initial begin
    @(negedge clk); // t=10
    a = 1; guard = 1;
    @(negedge clk); // t=20
    a = 0; guard = 1;
    @(negedge clk); // t=30
    a = 1; guard = 1;
    @(negedge clk); // t=40
    $finish;
  end

  p_always_active: assert property (@(posedge clk) a iff 1'b1)
    $display("T|iff_active|P t=%0t", $time);
  else
    $display("T|iff_active|F t=%0t", $time);

  // Test 3: nested iff (right-associative per LRM)
  // (a iff b) iff c  ≡  c |-> (b |-> a)
  initial begin
    @(negedge clk); // t=10
    a = 1; b = 1; guard = 1;
    @(negedge clk); // t=20
    a = 0; b = 1; guard = 1;
    @(negedge clk); // t=30
    a = 1; b = 0; guard = 1;
    @(negedge clk); // t=40
    $finish;
  end

  p_nested: assert property (@(posedge clk) a iff b iff guard)
    $display("T|iff_nested|P t=%0t", $time);
  else
    $display("T|iff_nested|F t=%0t", $time);

endmodule