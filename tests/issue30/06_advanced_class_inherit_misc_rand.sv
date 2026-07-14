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
// Abstract Base Class (Section 18.5.2)
// =============================================================================
virtual class AbstractBase;
  rand int base_var;
  rand int override_var;

  // Pure constraint represents a development obligation for child classes
  pure constraint implement_me;

  // Regular constraint to test standard inheritance rules
  constraint base_rules {
    base_var inside {[10 : 20]};
  }

  // Constraint to be overwritten in the next layer
  constraint dynamic_override {
    override_var == 5;
  }
endclass

// =============================================================================
// Mid-Level Abstract Class (Bypassing/Replacing Constraints)
// =============================================================================
virtual class AbstractMid extends AbstractBase;
  // An abstract class can replace an inherited constraint with a pure constraint
  pure constraint dynamic_override;
endclass

// =============================================================================
// Non-Abstract Concrete Class (Section 18.5.1 & 18.5.5 & 18.5.8)
// =============================================================================
class ConcreteEngine extends AbstractMid;
  // Scalar variables for uniqueness testing
  rand bit [7:0] unique_a;
  rand bit [7:0] unique_b;
  rand bit [7:0] unique_c;

  // Unpacked array variables for uniqueness and loop indexing bounds
  rand bit [7:0] unique_array[4];
  rand bit [7:0] matrix_2d[2][3]; // Multi-dimensional array mapping 
  rand bit [7:0] reduction_arr[];

  // 18.5.1 External Constraint Block Prototype
  constraint external_block_rule;

  // 18.5.2 Fulfilling inherited pure constraints from the abstract parent layer
  constraint implement_me {
    base_var == 15; // Refined constraint narrowing down the base scope
  }

  // 18.5.2 Overriding the pure constraint placeholder injected by AbstractMid
  constraint dynamic_override {
    override_var == 99;
  }

  // 18.5.5 Uniqueness Constraints over scalars and array slices
  constraint unique_group_rules {
    unique { unique_a, unique_b, unique_c, unique_array[0:2] };
  }

  // 18.5.8 Iterative Constraints (foreach and reduction loops)
  constraint iterative_rules {
    reduction_arr.size() == 5;

    // Dimension cardinality maps outer loop variable (i) to row, inner (j) to column
    foreach (matrix_2d[i, j]) {
      matrix_2d[i][j] == (i * 10) + j;
    }

    // Array reduction constraint method enforcing bounded summation
    reduction_arr.sum() with (int'(item)) == 100;
    foreach (reduction_arr[k]) {
      reduction_arr[k] inside {[10:30]};
    }
  }
endclass

// -----------------------------------------------------------------------------
// 18.5.1 Implementation of the External Constraint Block
// -----------------------------------------------------------------------------
constraint ConcreteEngine::external_block_rule {
  unique_a inside {[1:10]};
  unique_b inside {[11:20]};
  unique_c inside {[21:30]};
}

// =============================================================================
// Testbench Module Architecture
// =============================================================================
module tb_top;

  initial begin
    `SVTEST_INIT

    ConcreteEngine engine = new();
    int status;
    int actual_sum;

    repeat (20) begin
      status = engine.randomize();
      `SVTEST_CHECK(status == 1, "Randomization failed on inheritance and array loop matrices")

      // Verify Section 18.5.2: Inherited Base Constraints still functional
      `SVTEST_CHECK(engine.base_var >= 10 && engine.base_var <= 20, "Base range constraint dropped")

      // Verify Section 18.5.2: Pure Constraint implementation check
      `SVTEST_CHECK(engine.base_var == 15, "Pure constraint override resolution failed")

      // Verify Section 18.5.2: Intermediate pure constraint override success
      `SVTEST_CHECK(engine.override_var == 99, "Abstract mid-layer pure override failed to overwrite base value")

      // Verify Section 18.5.1: External block limits enforced correctly
      `SVTEST_CHECK(engine.unique_a >= 1 && engine.unique_a <= 10, "External constraint block bounds broken")

      // Verify Section 18.5.5: Uniqueness constraints (Scalar vs Array slices)
      `SVTEST_CHECK(engine.unique_a != engine.unique_b, "Uniqueness scalar collision encountered")
      `SVTEST_CHECK(engine.unique_b != engine.unique_c, "Uniqueness scalar collision encountered")
      `SVTEST_CHECK(engine.unique_a != engine.unique_array[0], "Uniqueness cross-array edge case failure")
      `SVTEST_CHECK(engine.unique_array[0] != engine.unique_array[1], "Uniqueness internal array slice error")

      // Verify Section 18.5.8.1: Multi-dimensional dimension cardinality mapping
      foreach (engine.matrix_2d[i, j]) begin
        `SVTEST_CHECK(engine.matrix_2d[i][j] == (i * 10) + j, "Foreach multi-dimensional loop mapping failed")
      end

      // Verify Section 18.5.8.2: Array reduction iterator validation
      actual_sum = 0;
      foreach (engine.reduction_arr[k]) begin
        `SVTEST_CHECK(engine.reduction_arr[k] >= 10 && engine.reduction_arr[k] <= 30, "Iterative array item out of bounds")
        actual_sum += engine.reduction_arr[k];
      end
      `SVTEST_CHECK(actual_sum == 100, "Array reduction sum math miscalculated by solver")
    end

    `SVTEST_PASSFAIL
  end

endmodule
