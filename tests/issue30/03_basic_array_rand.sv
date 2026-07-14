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
// Comprehensive Array Randomization Verification Class
// =============================================================================
class ArrayRandomizationBus;

  // ---------------------------------------------------------------------------
  // 1. Core Array Declarations (Original Section 18.3 Features)
  // ---------------------------------------------------------------------------
  rand  bit [7:0] fixed_rand_arr[4];
  randc bit [1:0] fixed_randc_arr[4]; 
  rand  bit [7:0] dyn_arr[];
  rand  bit [7:0] assoc_arr[bit [3:0]]; 
  rand  bit [7:0] test_queue[$];
  int index_offset = 10;

  // ---------------------------------------------------------------------------
  // 2. Multidimensional Dynamic Arrays (New)
  // ---------------------------------------------------------------------------
  // A dynamic outer array containing nested dynamic inner arrays
  rand bit [7:0] multi_dyn_arr[][];

  // ---------------------------------------------------------------------------
  // 3. Variables for Existential Array Reduction Constraints (New)
  // ---------------------------------------------------------------------------
  rand bit [7:0] reduction_arr[];
  rand int       array_sum_val;

  // ---------------------------------------------------------------------------
  // 4. Baseline Constraints (Original Sizing & Iterative Loops)
  // ---------------------------------------------------------------------------
  constraint array_sizes {
    dyn_arr.size()    inside {[3 : 6]};
    test_queue.size() inside {[2 : 4]};
  }

  constraint element_rules {
    foreach (fixed_rand_arr[i]) {
      fixed_rand_arr[i] inside {[10:50]};
    }
    foreach (dyn_arr[j]) {
      dyn_arr[j] == (j * 5) + index_offset; 
    }
    foreach (test_queue[k]) {
      test_queue[k] % 2 == 0; 
    }
  }

  // ---------------------------------------------------------------------------
  // 5. Advanced Constraints (New: Multidimensional Size & Iteration Loops)
  // ---------------------------------------------------------------------------
  constraint multidimensional_rules {
    // Constrain the length of the outer dynamic array
    multi_dyn_arr.size() == 3;

    // Constrain the length of each inner nested dynamic array individually
    foreach (multi_dyn_arr[i]) {
      multi_dyn_arr[i].size() == (i + 2); // Row 0 has 2 items, Row 1 has 3, Row 2 has 4
    }

    // Populate elements of the multidimensional matrix using iterative loop variables
    foreach (multi_dyn_arr[i, j]) {
      multi_dyn_arr[i][j] == (i * 10) + j;
    }
  }

  // ---------------------------------------------------------------------------
  // 6. Advanced Constraints (New: Existential Array Reductions)
  // ---------------------------------------------------------------------------
  constraint reduction_rules {
    reduction_arr.size() == 5;
    
    // Enforce value bounds for array elements to prevent bit overflow during summation
    foreach (reduction_arr[i]) {
      reduction_arr[i] inside {[1 : 10]};
    }

    // Existential verification using the built-in .sum() array reduction method
    array_sum_val == reduction_arr.sum() with (int'(item));
    array_sum_val inside {[20 : 40]};
  }

  // Helper mechanism to seed associative array keys prior to randomization
  function void pre_randomize();
    assoc_arr[4'h2] = 8'hAA;
    assoc_arr[4'h5] = 8'hBB;
    assoc_arr[4'hA] = 8'hCC;
  endfunction

endclass

// =============================================================================
// Testbench Module Architecture
// =============================================================================
module tb_top;

  initial begin
    `SVTEST_INIT

    ArrayRandomizationBus bus = new();
    int status;
    int expected_sum;

    repeat (20) begin
      status = bus.randomize();
      `SVTEST_CHECK(status == 1, "Array randomization solver failed")

      // -----------------------------------------------------------------------
      // Self-Checking Verification: Baseline Features
      // -----------------------------------------------------------------------
      foreach (bus.fixed_rand_arr[i]) begin
        `SVTEST_CHECK(bus.fixed_rand_arr[i] >= 10 && bus.fixed_rand_arr[i] <= 50,
                      "Fixed array element range failure")
      end

      `SVTEST_CHECK(bus.dyn_arr.size() >= 3 && bus.dyn_arr.size() <= 6, "dyn_arr size failure")
      foreach (bus.dyn_arr[j]) begin
        `SVTEST_CHECK(bus.dyn_arr[j] == (j * 5) + bus.index_offset, "dyn_arr math mismatch")
      end

      `SVTEST_CHECK(bus.assoc_arr[4'h2] != 8'hAA && bus.assoc_arr[4'h5] != 8'hBB && bus.assoc_arr[4'hA] != 8'hCC, 
                    "Associative elements were not overwritten")

      `SVTEST_CHECK(bus.test_queue.size() >= 2 && bus.test_queue.size() <= 4, "Queue size failure")
      foreach (bus.test_queue[k]) begin
        `SVTEST_CHECK(bus.test_queue[k] % 2 == 0, "Queue element mathematical violation")
      end

      // -----------------------------------------------------------------------
      // Self-Checking Verification: Multidimensional Dynamic Arrays
      // -----------------------------------------------------------------------
      $display("INFO: Validating Multidimensional Array Support...");
      `SVTEST_CHECK(bus.multi_dyn_arr.size() == 3, "Outer dynamic array size mismatch")
      
      foreach (bus.multi_dyn_arr[i]) begin
        `SVTEST_CHECK(bus.multi_dyn_arr[i].size() == (i + 2), "Inner nested dynamic array sizing error")
      end

      foreach (bus.multi_dyn_arr[i, j]) begin
        `SVTEST_CHECK(bus.multi_dyn_arr[i][j] == (i * 10) + j, "Multidimensional element evaluation mismatch")
      end

      // -----------------------------------------------------------------------
      // Self-Checking Verification: Existential Array Reductions
      // -----------------------------------------------------------------------
      $display("INFO: Validating Existential Array Reduction Support...");
      `SVTEST_CHECK(bus.reduction_arr.size() == 5, "Reduction target array size mismatch")

      expected_sum = 0;
      foreach (bus.reduction_arr[i]) begin
        `SVTEST_CHECK(bus.reduction_arr[i] >= 1 && bus.reduction_arr[i] <= 10, "Reduction item out of bounds")
        expected_sum += bus.reduction_arr[i];
      end

      `SVTEST_CHECK(bus.array_sum_val == expected_sum, "Solver array_sum_val does not match evaluated array sum")
      `SVTEST_CHECK(bus.array_sum_val >= 20 && bus.array_sum_val <= 40, "Existential summation bound violated")
    end

    `SVTEST_PASSFAIL
  end

endmodule
