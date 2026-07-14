`ifndef SVTEST_DEFS_SVH
`define SVTEST_DEFS_SVH

`define SVTEST_INIT \
  int failures = 0;

`define SVTEST_CHECK(expr, msg) \
  if (!(expr)) begin \
    failures++; \
    $display("FAIL: %s", msg); \
  end

`define SVTEST_PASSFAIL \
  if (failures == 0) begin \
    $display("TEST_PASS"); \
  end else begin \
    $display("TEST_FAIL count=%0d", failures); \
    $fatal(1); \
  end

`endif

// Class demonstrating dynamic weights and scope variables
class DynamicScopeBus;
  rand bit [7:0] data_val;
  
  // Non-random state variables used to dynamically alter distribution weights (18.10)
  int weight_low  = 10;
  int weight_high = 90;

  // Constraint block driven by dynamic variables rather than literal constants
  constraint dynamic_dist {
    data_val dist {
      8'h05       := weight_low,
      [8'hA0:8'hAF] := weight_high
    };
  }
endclass

module tb_top;

  initial begin
    `SVTEST_INIT

    DynamicScopeBus bus = new();
    int status;
    int data_val = 8'hA2; // Local testbench variable shadowing the class attribute name

    int high_range_hits = 0;
    int low_range_hits  = 0;

    // -------------------------------------------------------------------------
    // Test A: 18.7.1 local:: Scope Resolution
    // -------------------------------------------------------------------------
    // 'local::data_val' explicitly references the local testbench variable (8'hFF)
    // instead of shadowing or interacting with 'bus.data_val'
    status = bus.randomize() with { data_val == local::data_val; };
    `SVTEST_CHECK(status == 1, "Randomize with local:: scope modifier failed")
    `SVTEST_CHECK(bus.data_val == 8'hA2, "local:: scope resolution selected incorrect variable reference")


    // -------------------------------------------------------------------------
    // Test B: 18.10 Dynamic Constraint Modification (Dynamic Weights)
    // -------------------------------------------------------------------------
    // Configuration 1: High weight on upper range
    bus.weight_low  = 1;
    bus.weight_high = 99;
    
    repeat (50) begin
      status = bus.randomize();
      `SVTEST_CHECK(status == 1, "Randomization failed during high-weight pass")
      if (bus.data_val inside {[8'hA0:8'hAF]}) high_range_hits++;
    end
    `SVTEST_CHECK(high_range_hits > 40, "Dynamic weights failed to steer distribution to high range")

    // Configuration 2: High weight on lower range
    bus.weight_low  = 99;
    bus.weight_high = 1;
    
    repeat (50) begin
      status = bus.randomize();
      `SVTEST_CHECK(status == 1, "Randomization failed during low-weight pass")
      if (bus.data_val == 8'h05) low_range_hits++;
    end
    `SVTEST_CHECK(low_range_hits > 40, "Dynamic weights failed to steer distribution to low range")


    // -------------------------------------------------------------------------
    // Test C: 18.11 & 18.11.1 Randomize with Null (In-line Constraint Checker)
    // -------------------------------------------------------------------------
    // Calling .randomize(null) freezes ALL random variables in the object.
    // It treats existing data states as constants and acts as an in-line checker 
    // to see if the current object state complies with active or in-line constraints.
    
    // Scenario C1: Object state is valid (bus.data_val was set to 8'h05 in last run, which matches dist)
    bus.data_val = 8'h05;
    status = bus.randomize(null);
    `SVTEST_CHECK(status == 1, "Constraint checker falsely reported failure on a valid state")

    // Scenario C2: Force an illegal state. Checker must catch it and return 0.
    bus.data_val = 8'h55; // 8'h55 is outside the active 'dynamic_dist' constraint
    status = bus.randomize(null);
    `SVTEST_CHECK(status == 0, "Constraint checker failed to catch an illegal data value")

    // Scenario C3: Verify it works when paired with custom inline constraints
    bus.data_val = 8'hA5; // Historically valid under 'dynamic_dist'
    status = bus.randomize(null) with { data_val == 8'hA5; };
    `SVTEST_CHECK(status == 1, "Inline constraint checker rejected a valid state comparison")

    status = bus.randomize(null) with { data_val == 8'h00; };
    `SVTEST_CHECK(status == 0, "Inline constraint checker failed to catch an un-matched condition")

    // -------------------------------------------------------------------------
    // Report Results
    // -------------------------------------------------------------------------
    `SVTEST_PASSFAIL
  end

endmodule
