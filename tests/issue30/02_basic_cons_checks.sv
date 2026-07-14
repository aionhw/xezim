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

// Class exercising Section 18.3 variable constraints and advanced solver math
class AdvancedEquationBus;

  // 1. Core Integral Variable Types (as specified in Section 18.3)
  typedef enum bit [1:0] { START, DATA, STOP } type_e;
  typedef struct packed {
    bit        valid;
    bit [2:0]  tag;
  } header_s;

  rand bit [4:0]    shift_amt; // 5-bit variable for power-of-two testing
  rand bit [31:0]   power_of_two;
  rand reg [7:0]    r_val;
  rand logic [7:0]  l_val;
  rand integer      int_val;   // signed 32-bit type
  rand type_e       pkt_type;  // enum type
  rand header_s     pkt_hdr;   // packed struct type

  // New variables for Division and Modulus testing
  rand bit [7:0]    div_num;
  rand bit [7:0]    div_den;
  rand bit [7:0]    div_res;
  rand bit [7:0]    mod_val;
  rand bit [7:0]    mod_res;

  // New variables for Distribution (dist) operator testing
  rand bit [3:0]    dist_eq_weight;
  rand bit [7:0]    dist_prop_weight;

  // 2. Algebraic Factoring Equation
  constraint algebraic_factoring {
    (r_val - 8'd10) * (l_val + 8'd5) == 16'd0;
    r_val inside {[10:20]}; 
  }

  // 3. Complex Boolean Expressions
  constraint complex_boolean {
    (pkt_type == START) -> (pkt_hdr.valid == 1'b1 && pkt_hdr.tag > 3'd4);
    (pkt_type == STOP)  -> (pkt_hdr.valid == 1'b0 && pkt_hdr.tag == 3'd0);
    ((pkt_type == DATA) && pkt_hdr.valid) || (int_val < 0);
  }

  // 4. Mixed Integer and Bit Expressions
  constraint mixed_types {
    int_val inside {[-100 : 100]};
    int_val + $signed({1'b0, l_val}) == 32'd50;
  }

  // 5. Power-of-two constraint using a shift operator (1 << n)
  constraint power_of_two_shift {
    power_of_two == (32'd1 << shift_amt);
  }

  // 6. Division and Modulus algebraic constraints
  constraint div_mod_operations {
    div_den != 8'd0;                        // Guard against zero-division exceptions
    div_num inside {[8'd50 : 8'd100]};
    div_den inside {[8'd2  : 8'd10]};
    
    div_res == div_num / div_den;           // Integer division execution
    mod_res == mod_val % 8'd5;              // Modulus reduction operation
    mod_val inside {[8'd10 : 8'd30]};
  }

  // 7. Weighted Distribution Rules
  constraint distributions {
    // ':=' operator assigns identical specified weights to individual items in a range
    dist_eq_weight dist {
      4'h0       := 40, 
      [4'h1:4'h3] := 20  // 1, 2, and 3 each receive an exact weight of 20
    };

    // ':/' operator divides specified weight proportionally across the items in a range
    dist_prop_weight dist {
      8'd100        := 50,
      [8'd0:8'd4]   :/ 50  // Total range weight is 50 (each element 0-4 gets a weight of 10)
    };
  }

endclass

// =============================================================================
// Verification Testbench
// =============================================================================
module tb_top;

  initial begin
    `SVTEST_INIT

    AdvancedEquationBus bus = new();
    int status;

    // Distribution tracking state fields
    int eq_weight_0_count = 0;
    int eq_weight_range_count = 0;
    int prop_weight_100_count = 0;
    int prop_weight_range_count = 0;

    // Execute 500 loops to allow statistical patterns to form for dist verification
    repeat (500) begin
      status = bus.randomize();
      `SVTEST_CHECK(status == 1, "Solver failed to resolve advanced expression combinations")

      // -----------------------------------------------------------------------
      // Self-Checking Verification Loops
      // -----------------------------------------------------------------------

      // Verify Test 1: Algebraic Factoring
      `SVTEST_CHECK(bus.r_val == 8'd10, "Algebraic factoring constraint failed")

      // Verify Test 2: Shift Operator and Power-of-Two Validation
      `SVTEST_CHECK($onehot(bus.power_of_two), "Shift expression failed to generate a power of two")
      `SVTEST_CHECK(bus.power_of_two == (32'd1 << bus.shift_amt), "Power of two mismatch relative to shift amount")

      // Verify Test 3: Complex Boolean System Verification
      if (bus.pkt_type == AdvancedEquationBus::START) begin
        `SVTEST_CHECK(bus.pkt_hdr.valid == 1'b1, "Implication failure: valid bit false during START")
        `SVTEST_CHECK(bus.pkt_hdr.tag > 3'd4, "Implication failure: tag bounds broken during START")
      end
      if (bus.pkt_type == AdvancedEquationBus::STOP) begin
        `SVTEST_CHECK(bus.pkt_hdr.valid == 1'b0, "Implication failure: valid bit true during STOP")
        `SVTEST_CHECK(bus.pkt_hdr.tag == 3'd0, "Implication failure: tag non-zero during STOP")
      end

      // Verify Test 4: Mixed Sign and Expression Length Integrity
      `SVTEST_CHECK(bus.int_val >= -100 && bus.int_val <= 100, "Integer out of safety test boundaries")
      `SVTEST_CHECK(bus.int_val + $signed({1'b0, bus.l_val}) == 32'd50, "Mixed integer/bit logic arithmetic miscalculated")

      // Verify Test 5: Division and Modulus Arithmetic
      `SVTEST_CHECK(bus.div_den != 8'd0, "Zero division safety boundary breached")
      `SVTEST_CHECK(bus.div_res == (bus.div_num / bus.div_den), "Solver division operation output miscalculated")
      `SVTEST_CHECK(bus.mod_res == (bus.pkt_hdr.valid ? (bus.mod_val % 8'd5) : (bus.mod_val % 8'd5)), "Solver modulus calculation failed")

      // Verify Test 6: Distribution State Bound Traps
      `SVTEST_CHECK(bus.dist_eq_weight inside {4'h0, [4'h1:4'h3]}, "Equal weight dist generated value outside specification bounds")
      `SVTEST_CHECK(bus.dist_prop_weight inside {8'd100, [8'd0:8'd4]}, "Proportional weight dist value outside bounds")

      // Accumulate hits for statistical verification check 
      if (bus.dist_eq_weight == 4'h0)      eq_weight_0_count++;
      if (bus.dist_eq_weight inside {[1:3]}) eq_weight_range_count++;
      if (bus.dist_prop_weight == 8'd100)   prop_weight_100_count++;
      if (bus.dist_prop_weight inside {[0:4]}) prop_weight_range_count++;
    end

    // Verify Test 7: Distribution Hit Sanity
    // Ensure all defined distribution targets were generated across the verification loop
    `SVTEST_CHECK(eq_weight_0_count > 0, "Equal weight single distribution hit target 0 was missed")
    `SVTEST_CHECK(eq_weight_range_count > 0, "Equal weight range distribution target loop missed completely")
    `SVTEST_CHECK(prop_weight_100_count > 0, "Proportional weight single distribution target 100 missed")
    `SVTEST_CHECK(prop_weight_range_count > 0, "Proportional weight range distribution missed completely")

    // -----------------------------------------------------------------------
    // Report Test Success or Failure
    // -----------------------------------------------------------------------
    `SVTEST_PASSFAIL
  end

endmodule
