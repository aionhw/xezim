//! Issue #1204: per-driver delays on a user-defined-nettype net. Each
//! continuous driver of a resolved net keeps its OWN delay (§10.3.3): a
//! `#2` driver's contribution stays at the nettype's 0.0 initial for its
//! first delay period while an undelayed driver contributes immediately,
//! and the resolution function re-runs the moment any single driver's
//! contribution changes. A nettype net's declaration initializer is itself
//! a driver, so a resolver keyed on driver COUNT sees it.
//! Expected values were checked byte-for-byte against the reference
//! simulators.
module top;
  int fails;
  task check(string tag, logic ok);
    if (!ok) begin $display("FAIL: %s", tag); fails++; end
  endtask

  function automatic real sum(input real d[]);
    real r = 0.0; foreach (d[i]) r += d[i]; return r;
  endfunction
  nettype real N with sum;

  real a = 1.25, b = 2.5, c = 3.0;
  N n, n2, n3;           // no declaration initializer
  real e1, e2, e3;       // expected witness values

  // Delayed driver + undelayed driver (issue #1204 main case).
  assign #2 n = a;
  assign n = b;

  // Same-delay drivers must NOT be staged (uniform delay is one fold).
  assign #2 n2 = a;
  assign #2 n2 = b;

  // Three drivers with three distinct delays.
  assign #2 n3 = a; assign #3 n3 = b; assign n3 = c;

  // A nettype net whose DECLARATION initializer is a driver of the count.
  function automatic real cnt3(input real d[]);
    if (d.size() != 3) $fatal(1, "expected 3 drivers, got %0d", d.size());
    return d[0] + d[1] + d[2];
  endfunction
  nettype real C with cnt3;
  C m = 0.0; real d2 = 1.0, e = 2.0;
  assign #2 m = d2;
  assign m = e;

  initial begin
    #1; // t=1
    check("main_t1", n === 2.5);   // 0.0 (delayed) + 2.5
    check("uniform_t1", n2 === 0.0); // both delayed -> both 0
    check("multi_t1", n3 === 3.0);  // 0.0 + 0.0 + 3.0
    check("declinit_t1", m === 2.0); // 0.0 + 2.0 + 0.0(decl init)
    #2; // t=3
    check("main_t3", n === 3.75);  // 1.25 + 2.5
    check("uniform_t3", n2 === 3.75); // 1.25 + 2.5
    check("multi_t3", n3 === 6.75);  // 1.25 + 2.5 + 3.0
    check("declinit_t3", m === 3.0); // 1.0 + 2.0 + 0.0
    a = 3.25; #1; // t=4 (a changes at 3)
    check("main_t4", n === 3.75);  // a delayed: still 1.25 + 2.5
    check("multi_t4", n3 === 6.75); // a delayed #2 -> 3.25 only at t5
    #2; // t=6
    check("main_t6", n === 5.75);  // 3.25 + 2.5
    check("multi_t6", n3 === 8.75);  // 3.25 + 2.5 + 3.0
    if (fails == 0) $display("PASS");
    $finish;
  end
  initial #100 $finish;
endmodule
