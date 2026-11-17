//! Issue #1239: a chained nettype relay misses a later update.
//!
//! `always @(n0.f or n0.v) m1 = n0.f; assign n1 = mk(m1);` then
//! `always @(n1.f or n1.v) m2 = n1.f; assign n2 = mk(m2);` — the second
//! relay's input (n1.f) changes only in the final settle that propagates the
//! first relay's blocking write, after the cascade loop's last check_edges.
//! That settle-time edge must re-trigger the downstream relay IN THE SAME
//! timestep (IEEE §9.4.2), not sit stale until the next clock tick.
//! Expected values were checked byte-for-byte against the reference
//! simulators, including a 3-level chain and a second update.
package p;
  typedef struct { real f; bit v; } rec;
  function automatic rec mk(real f); rec r; r.f = f; r.v = 1; return r; endfunction
  function automatic rec res(input rec d[]);
    rec r; r.f = 0.0; r.v = 0;
    foreach (d[i]) if (d[i].v) r = d[i];
    return r;
  endfunction
  nettype rec cnet with res;
endpackage

module top;
  import p::*;
  int fails;
  task check(string tag, logic ok);
    if (!ok) begin $display("FAIL: %s", tag); fails++; end
  endtask

  real s = 1.25, m1 = 0.0, m2 = 0.0, m3 = 0.0;
  cnet n0, n1, n2, n3;
  assign n0 = mk(s);
  always @(n0.f or n0.v) m1 = n0.f;
  assign n1 = mk(m1);
  always @(n1.f or n1.v) m2 = n1.f;
  assign n2 = mk(m2);
  always @(n2.f or n2.v) m3 = n2.f;
  assign n3 = mk(m3);

  initial begin
    #1 s = 2.75;
    #1; // t=2: relay chain settles within the timestep
    check("chain2", n2.f === 2.75); // second relay sees n1.f=2.75 same-delta
    check("chain3", n3.f === 2.75); // third relay sees n2.f=2.75 same-delta
    #2 s = 4.0;
    #1; // second update must re-propagate through the whole chain
    check("chain2b", n2.f === 4.0);
    check("chain3b", n3.f === 4.0);
    if (fails == 0) $display("PASS");
    $finish;
  end
  initial #100 $finish;
endmodule
