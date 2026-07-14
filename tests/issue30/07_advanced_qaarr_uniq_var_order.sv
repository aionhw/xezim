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

// =============================================================================
// Helper Object Class for Global Constraints Testing (18.5.9)
// =============================================================================
class SubObject;
  rand bit [7:0] sub_data;
endclass

// =============================================================================
// Base Variable Ordering Configuration Sandbox (18.5.10)
// =============================================================================
class OrderingBase;
  rand bit       control_bit;
  rand bit [7:0] data_byte;

  // Implication logic causing mathematical solution space distortion
  constraint implication_distortion {
    (control_bit == 1'b0) -> (data_byte == 8'h00);
  }
endclass

// Subclass implementing ordering priority configuration
class OrderedSub extends OrderingBase;
  // Forces uniform selection priority over control state space
  constraint structural_order {
    solve control_bit before data_byte;
  }
endclass

// =============================================================================
// Primary LRM Test Container Class
// =============================================================================
class GlobalAdvancedBus;

  // 18.5.9 Global Constraints - Enclosing instance handle solver setup
  rand SubObject sub_inst;

  // Collection variables for uniqueness constraint validation
  rand bit [7:0] test_queue[$];
  rand bit [7:0] test_assoc[int];

  // 18.5.11 Static Constraint Blocks 
  static constraint static_range_block {
    foreach (test_queue[i]) {
      test_queue[i] inside {[1 : 50]};
    }
  }

  // Uniqueness matrix execution over heterogeneous structural arrays
  constraint unique_collection_rules {
    test_queue.size() == 4;
    unique { test_queue }; // Enforce complete internal item divergence inside queue

    // Enforce value divergence inside associative array elements
    unique { test_assoc }; 
  }

  // 18.5.9 Global Cross-Object Constraint block linking instance variables
  constraint global_cross_rules {
    // Cross-boundary equation resolved simultaneously during execution
    sub_inst.sub_data == test_queue[0] + 8'd5;
  }

  function new();
    sub_inst = new();
    
    // Seed key map inside associative arrays so elements exist to randomize
    test_assoc[100] = 0;
    test_assoc[200] = 0;
    test_assoc[300] = 0;
  endfunction

endclass

// =============================================================================
// Testbench Module Architecture
// =============================================================================
module tb_top;

  initial begin
    `SVTEST_INIT

    GlobalAdvancedBus bus_a = new();
    GlobalAdvancedBus bus_b = new();
    
    OrderingBase non_ordered_obj = new();
    OrderedSub   ordered_obj     = new();
    
    int status;
    int non_ordered_zero_ctrl_hits = 0;
    int ordered_zero_ctrl_hits     = 0;

    // -------------------------------------------------------------------------
    // Test 1: Advanced Collection Uniqueness & Global Constraints
    // -------------------------------------------------------------------------
    repeat (10) begin
      status = bus_a.randomize();
      `SVTEST_CHECK(status == 1, "Randomization failed on collection matrix structures")

      // Verify Section 18.5.9: Simultaneous global object assignment execution
      `SVTEST_CHECK(bus_a.sub_inst.sub_data == bus_a.test_queue[0] + 8'd5, 
                    "Global cross-object concurrent variable solver failed")

      // Verify Queue Uniqueness bounds
      `SVTEST_CHECK(bus_a.test_queue[0] != bus_a.test_queue[1], "Queue index value collision encountered")
      `SVTEST_CHECK(bus_a.test_queue[1] != bus_a.test_queue[2], "Queue index value collision encountered")
      `SVTEST_CHECK(bus_a.test_queue[2] != bus_a.test_queue[3], "Queue index value collision encountered")

      // Verify Associative Array Uniqueness boundaries
      `SVTEST_CHECK(bus_a.test_assoc[100] != bus_a.test_assoc[200], "Associative element allocation collision")
      `SVTEST_CHECK(bus_a.test_assoc[200] != bus_a.test_assoc[300], "Associative element allocation collision")
    end

    // -------------------------------------------------------------------------
    // Test 2: Static Constraint Blocks (18.5.11)
    // -------------------------------------------------------------------------
    // Query baseline constraints status (Should start active)
    `SVTEST_CHECK(bus_a.static_range_block.constraint_mode() == 1, "Static block baseline down")

    // Turn off static block via instance variable a
    bus_a.static_range_block.constraint_mode(0);

    // Verify it changed state globally across separate unrelated instance allocations
    `SVTEST_CHECK(bus_b.static_range_block.constraint_mode() == 0, 
                  "LRM Violation: Disabling static block on instance A failed to update instance B")

    // Randomize completely past old static boundaries to verify deactivation success
    status = bus_a.randomize() with { test_queue[0] == 8'd99; };
    `SVTEST_CHECK(status == 1, "Randomization failed with deactivated static layout")
    `SVTEST_CHECK(bus_a.test_queue[0] == 8'd99, "Static rule remained active incorrectly")

    // Restore sanity block states
    bus_b.static_range_block.constraint_mode(1);

    // -------------------------------------------------------------------------
    // Test 3: Variable Ordering Optimization Evaluation (18.5.10)
    // -------------------------------------------------------------------------
    // Profile statistical execution distribution shifts across 100 iterations
    repeat (100) begin
      void'(non_ordered_obj.randomize());
      void'(ordered_obj.randomize());

      if (non_ordered_obj.control_bit == 1'b0) non_ordered_zero_ctrl_hits++;
      if (ordered_obj.control_bit     == 1'b0) ordered_zero_ctrl_hits++;
    end

    // STATISTICAL PROFILE CHECKS:
    // Bi-directional distribution without hint tracking: zero hits should be mathematically extremely rare (<5 hits).
    // Prioritized variable ordering distribution: zero hits should hover safely around the uniform target zone (~50 hits).
    `SVTEST_CHECK(non_ordered_zero_ctrl_hits < 10, "Bi-directional distribution failed to isolate low probability branch output")
    `SVTEST_CHECK(ordered_zero_ctrl_hits     > 30, "Solve...before ordering priority failed to optimize variable distribution bounds")

    $display("INFO: Distribution Hit Metrics Profile -> Non-Ordered Zero Hits: %0d, Ordered Zero Hits: %0d", 
             non_ordered_zero_ctrl_hits, ordered_zero_ctrl_hits);

    // -------------------------------------------------------------------------
    // Report Test Success or Failure
    // -------------------------------------------------------------------------
    `SVTEST_PASSFAIL
  end

endmodule
