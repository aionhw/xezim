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
// 18.5.14.2 & 18.5.14.3: Base Layer for Hierarchy Resolution
// =============================================================================
class BasePacket;
  rand bit [7:0] len;
  rand bit [7:0] val;

  // Base soft constraints
  constraint base_soft_rules {
    soft len == 10;
    soft val inside {[0 : 50]};
  }
endclass

// Subclass demonstrating that trailing constraints override leading ones (18.5.14.2)
class DerivedPacket extends BasePacket;
  constraint derived_soft_rules {
    // Overrides the base len constraint because it is lower in the hierarchy
    soft len == 20; 
  }
endclass

// =============================================================================
// Primary LRM Test Container Class
// =============================================================================
class GuardAndSoftContainer;

  // Non-random state variables acting as Constraint Guards (18.5.13)
  bit mode_active = 0;
  int target_type = 0;

  // Random variables to test guards and soft constraints
  rand bit [7:0] guarded_data;
  rand bit [7:0] dynamic_threshold;

  // 18.5.13: Constraint Guards
  // The condition uses non-random variables. It acts as a guard, meaning if it
  // evaluates to false, the enclosed constraint is completely ignored by the solver.
  constraint guard_example_block {
    if (mode_active) {
      if (target_type == 1) {
        guarded_data inside {[100 : 200]};
      } else {
        guarded_data inside {[10 : 20]};
      }
    } else {
      guarded_data == 0;
    }
  }

  // 18.5.14: Basic Soft Constraint Block
  constraint soft_baseline {
    soft dynamic_threshold == 50;
  }

endclass


// =============================================================================
// Testbench Module Architecture
// =============================================================================
module tb_top;

  initial begin
    `SVTEST_INIT

    GuardAndSoftContainer container = new();
    DerivedPacket         pkt       = new();
    int status;

    // -------------------------------------------------------------------------
    // Test 1: Verify Section 18.5.13 - Constraint Guards
    // -------------------------------------------------------------------------
    
    // Scenario 1A: Guard is false (mode_active = 0)
    container.mode_active = 0;
    status = container.randomize();
    `SVTEST_CHECK(status == 1, "Guarded randomization failed")
    `SVTEST_CHECK(container.guarded_data == 0, "Guard condition false failed to enforce fallback branch")

    // Scenario 1B: Guard branch 1 is true (mode_active = 1, target_type = 1)
    container.mode_active = 1;
    container.target_type = 1;
    status = container.randomize();
    `SVTEST_CHECK(status == 1, "Guarded randomization failed")
    `SVTEST_CHECK(container.guarded_data >= 100 && container.guarded_data <= 200, "Nested guard path 1 failed")

    // Scenario 1C: Guard branch 2 is true (mode_active = 1, target_type = 0)
    container.mode_active = 1;
    container.target_type = 0;
    status = container.randomize();
    `SVTEST_CHECK(status == 1, "Guarded randomization failed")
    `SVTEST_CHECK(container.guarded_data >= 10 && container.guarded_data <= 20, "Nested guard path 2 failed")


    // -------------------------------------------------------------------------
    // Test 2: Verify Section 18.5.14.1 - Inline Constraints Overriding Soft
    // -------------------------------------------------------------------------
    
    // Scenario 2A: No conflict, soft constraint should be satisfied
    status = container.randomize();
    `SVTEST_CHECK(status == 1, "Soft baseline randomization failed")
    `SVTEST_CHECK(container.dynamic_threshold == 50, "Soft baseline failed to resolve to its default value")

    // Scenario 2B: Inline hard constraint overrides the soft constraint without error
    status = container.randomize() with { dynamic_threshold == 75; };
    `SVTEST_CHECK(status == 1, "Hard inline constraint failed to override the soft constraint")
    `SVTEST_CHECK(container.dynamic_threshold == 75, "Value did not update to inline hard override")


    // -------------------------------------------------------------------------
    // Test 3: Verify 18.5.14.2 & 18.5.14.3 - Hierarchical Soft Discrepancies
    // -------------------------------------------------------------------------
    
    // Scenario 3A: Derived class soft constraint must override base class soft constraint
    status = pkt.randomize();
    `SVTEST_CHECK(status == 1, "Hierarchical class randomization failed")
    `SVTEST_CHECK(pkt.len == 20, "LRM Violation: Derived soft constraint failed to override base layer")
    `SVTEST_CHECK(pkt.val >= 0 && pkt.val <= 50, "Non-conflicting base soft constraint was accidentally dropped")

    // Scenario 3B: Hard inline constraints can comfortably override any layer of the soft hierarchy
    status = pkt.randomize() with { len == 200; val == 222; };
    `SVTEST_CHECK(status == 1, "Inline hard constraints failed against class soft hierarchy")
    `SVTEST_CHECK(pkt.len == 200, "Inline hard failed to discard derived soft configuration")
    `SVTEST_CHECK(pkt.val == 222, "Inline hard failed to discard base soft configuration")

    // -------------------------------------------------------------------------
    // Report Final Results
    // -------------------------------------------------------------------------
    `SVTEST_PASSFAIL
  end

endmodule
