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
// Base Class Setup (Section 18.3 Layout)
// =============================================================================
class Section183Bus;
  rand bit [15:0] addr;
  rand bit [31:0] data;
  randc bit [1:0] cyclic_id;

  // Base behavior constraints
  constraint word_align {
    addr[1:0] == 2'b0;
  }

  constraint data_payload_bounds {
    data >= 32'h1000_0000;
    data <= 32'h2000_0000;
  }
endclass

// =============================================================================
// Extended Subclass (Inherited Layering, Call Hooks, and State Modifiers)
// =============================================================================
class MyBus extends Section183Bus;
  typedef enum bit [1:0] { LOW_RANGE, MID_RANGE, HIGH_RANGE } range_e;
  rand range_e bus_type;

  // Verification hooks tracking indicators
  int pre_count = 0;
  int post_count = 0;

  // New sub-class constraint built upon base class properties via inheritance
  constraint addr_range_layering {
    if (bus_type == LOW_RANGE) {
      addr inside {[16'h0000 : 16'h3FFF]};
    } else if (bus_type == MID_RANGE) {
      addr inside {[16'h4000 : 16'h7FFF]};
    } else {
      addr inside {[16'h8000 : 16'hFFFF]};
    }
  }

  // Hook automatically evaluated immediately before solver execution
  function void pre_randomize();
    pre_count++;
  endfunction

  // Hook automatically evaluated immediately following solver execution
  function void post_randomize();
    post_count++;
  endfunction
endclass

// =============================================================================
// Testbench Module Architecture
// =============================================================================
module tb_top;

  initial begin
    `SVTEST_INIT

    MyBus bus = new();
    int status;

    // -------------------------------------------------------------------------
    // Test 1: Verify Constraint Inheritance & Execution Hooks
    // -------------------------------------------------------------------------
    repeat (20) begin
      status = bus.randomize();
      `SVTEST_CHECK(status == 1, "Randomize invocation failed on extended class")

      // Verify base class constraints are still fully active (Inherited)
      `SVTEST_CHECK(bus.addr[1:0] == 2'b0, "Base alignment constraint broke in child class")
      `SVTEST_CHECK(bus.data >= 32'h1000_0000 && bus.data <= 32'h2000_0000, "Base range broke")

      // Verify extended subclass layering constraints are correctly enforced
      if (bus.bus_type == MyBus::LOW_RANGE) begin
        `SVTEST_CHECK(bus.addr <= 16'h3FFF, "Low range constraint violation")
      end else if (bus.bus_type == MyBus::MID_RANGE) begin
        `SVTEST_CHECK(bus.addr >= 16'h4000 && bus.addr <= 16'h7FFF, "Mid range constraint violation")
      end else begin
        `SVTEST_CHECK(bus.addr >= 16'h8000, "High range constraint violation")
      end
    end

    // Verify hooks executed exactly 20 times matching the execution loop iteration
    `SVTEST_CHECK(bus.pre_count == 20, "pre_randomize execution tracker mismatch")
    `SVTEST_CHECK(bus.post_count == 20, "post_randomize execution tracker mismatch")

    // -------------------------------------------------------------------------
    // Test 2: Verify constraint_mode() Deactivation & Reactivation
    // -------------------------------------------------------------------------
    // Query initial state via function call syntax (Must return 1 for enabled)
    `SVTEST_CHECK(bus.addr_range_layering.constraint_mode() == 1, "Constraint state should be active")

    // Disable extended constraint block completely
    bus.addr_range_layering.constraint_mode(0);
    `SVTEST_CHECK(bus.addr_range_layering.constraint_mode() == 0, "Deactivation failed")

    // Force data into LOW_RANGE via inline constraint, but allow address to break bounds
    status = bus.randomize() with { bus_type == LOW_RANGE; };
    `SVTEST_CHECK(status == 1, "Randomization failed with disabled subclass constraint")
    `SVTEST_CHECK(bus.addr[1:0] == 2'b0, "Base constraints must remain active")

    // Since child range constraint is disabled, address should be free to roll outside [0:16'h3FFF]
    // We execute multiple times with an inline constraint forcing a high block address
    status = bus.randomize() with { bus_type == LOW_RANGE; addr == 16'hF000; };
    `SVTEST_CHECK(status == 1, "Deactivated constraint was unexpectedly enforced")
    `SVTEST_CHECK(bus.addr == 16'hF000, "Subclass constraint was not bypassed")

    // Restore constraint state back to normal configuration
    bus.addr_range_layering.constraint_mode(1);
    `SVTEST_CHECK(bus.addr_range_layering.constraint_mode() == 1, "Reactivation failed")

    // -------------------------------------------------------------------------
    // Test 3: Verify rand_mode() Variable Randomization Control
    // -------------------------------------------------------------------------
    // Check baseline property status (Must return 1 for actively randomized variables)
    `SVTEST_CHECK(bus.data.rand_mode() == 1, "Variable should look random by default")

    // Establish fixed constants and strip down the random trait from 'data' variable
    bus.data = 32'h1A2B_3C4D;
    bus.data.rand_mode(0);
    `SVTEST_CHECK(bus.data.rand_mode() == 0, "Variable state transition failed")

    // Randomize multiple times; the non-random state variable must hold its static value
    repeat (10) begin
      status = bus.randomize();
      `SVTEST_CHECK(status == 1, "Randomize operation crashed with disabled variable")
      `SVTEST_CHECK(bus.data == 32'h1A2B_3C4D, "Disabled rand variable shifted value out-of-turn")
    end

    // Re-enable variable randomization functionality
    bus.data.rand_mode(1);
    `SVTEST_CHECK(bus.data.rand_mode() == 1, "Variable recovery state adjustment failed")

    // Verify it updates and randomizes away from its old constant baseline again
    status = bus.randomize() with { data != 32'h1A2B_3C4D; };
    `SVTEST_CHECK(status == 1, "Randomization failed after variable reactivation")
    `SVTEST_CHECK(bus.data != 32'h1A2B_3C4D, "Variable did not update value when randomized")

    // -------------------------------------------------------------------------
    // Report Test Results
    // -------------------------------------------------------------------------
    `SVTEST_PASSFAIL
  end

endmodule