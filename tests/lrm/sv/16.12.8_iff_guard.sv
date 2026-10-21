// top: t16_12_8_iff_guard
// LRM §16.12.8: property_expr iff (guard_expr) — vacuous pass when guard is false
module t16_12_8_iff_guard;
  bit clk;
  always #5 clk = ~clk;

  // Test 1: basic boolean iff
  bit a1, guard1;
  initial begin
    @(negedge clk); // t=10
    a1 = 1;
    @(negedge clk); // t=20
    a1 = 0; guard1 = 1;
    @(negedge clk); // t=30
    a1 = 1;
    @(negedge clk); // t=40
    a1 = 0; guard1 = 0;
    @(negedge clk); // t=50
    guard1 = 1;     // a1=0
    @(negedge clk); // t=60
    guard1 = 0;
    @(negedge clk); // t=70
    $finish;
  end

  p1: assert property (@(posedge clk) a1 iff guard1)
    $display("T|iff_basic|P t=%0t", $time);
  else
    $display("T|iff_basic|F t=%0t", $time);

  // Test 2: implication as property: (a |-> b) iff guard
  bit a2, b2, guard2;
  initial begin
    @(negedge clk); // t=10
    a2 = 1; b2 = 0; guard2 = 0;
    @(negedge clk); // t=20
    a2 = 0; b2 = 1; guard2 = 1;
    @(negedge clk); // t=30
    a2 = 1; b2 = 1; guard2 = 1;
    @(negedge clk); // t=40
    a2 = 0; b2 = 0; guard2 = 0;
    @(negedge clk); // t=50
    a2 = 1; b2 = 0; guard2 = 1;
    @(negedge clk); // t=60
    guard2 = 0;
    @(negedge clk); // t=70
    // $finish handled by test 1
  end

  p2: assert property (@(posedge clk) (a2 |-> b2) iff guard2)
    $display("T|iff_impl|P t=%0t", $time);
  else
    $display("T|iff_impl|F t=%0t", $time);

endmodule