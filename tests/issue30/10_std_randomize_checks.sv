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

module tb_top;

  // Packed structure used for integral testing
  typedef struct packed {
    bit [3:0] field_a;
    bit [3:0] field_b;
  } packed_struct_s;

  initial begin
    `SVTEST_INIT

    int status;

    // -------------------------------------------------------------------------
    // Standard Local Scope Procedural Variables
    // -------------------------------------------------------------------------
    bit [15:0]      scalar_addr;
    bit [31:0]      scalar_data;
    bit [7:0]       div_num, div_den, div_res;
    bit [3:0]       dist_val;
    bit [7:0]       unique_queue[$], unique_dynarr[];
    packed_struct_s p_struct;

    // -------------------------------------------------------------------------
    // Scenario 1: Basic Variables, Algebraic Factoring, Mixed Expressions
    // -------------------------------------------------------------------------
    status = std::randomize(scalar_addr, scalar_data) with {
      scalar_addr[1:0] == 2'b0; // Word alignment check
      (scalar_data - 32'd10) * (scalar_data - 32'd20) == 32'd0; // Algebraic factoring
      scalar_data == 32'd20; // Narrowing the solution down
    };
    `SVTEST_CHECK(status == 1, "std::randomize scenario 1 failed")
    `SVTEST_CHECK(scalar_addr[1:0] == 2'b0, "Alignment constraint broke in std::randomize")
    `SVTEST_CHECK(scalar_data == 32'd20, "Algebraic factoring broke in std::randomize")


    // -------------------------------------------------------------------------
    // Scenario 2: Division and Modulus Arithmetic Primitives
    // -------------------------------------------------------------------------
    status = std::randomize(div_num, div_den, div_res) with {
      div_den != 8'd0;
      div_num inside {[8'd20 : 8'd40]};
      div_den == 8'd5;
      div_res == div_num / div_den;
      div_num % 8'd3 == 8'd0; // Add Modulus criteria to resolution space
    };
    `SVTEST_CHECK(status == 1, "std::randomize scenario 2 failed")
    `SVTEST_CHECK(div_den == 8'd5, "Denom boundary resolution failed")
    `SVTEST_CHECK(div_res == (div_num / div_den), "Division logic failed inside scope constraint")
    `SVTEST_CHECK(div_num % 8'd3 == 0, "Modulus parameter constraint failed")


    // -------------------------------------------------------------------------
    // Scenario 3: Weight-Driven Allocation Distributions (dist)
    // -------------------------------------------------------------------------
    status = std::randomize(dist_val) with {
      dist_val dist { 4'hA := 100, [4'h0:4'h4] := 0 };
    };
    `SVTEST_CHECK(status == 1, "std::randomize scenario 3 failed")
    `SVTEST_CHECK(dist_val == 4'hA, "Distribution rule failed to select forced target weight")


    // -------------------------------------------------------------------------
    // Scenario 4: Dynamic Collections Sizing & Uniqueness Verification
    // -------------------------------------------------------------------------
    // Seed size prior to invoking scope randomization 
    unique_dynarr = new[4];
    unique_queue = '{0,0,0,0};

    status = std::randomize(unique_queue, unique_dynarr) with {
      unique { unique_queue }; // Enforce element separation across procedural queue arrays
      unique { unique_dynarr }; 
      foreach (unique_queue[i]) {
        unique_queue[i] inside {[1:10]};
      }
        foreach (unique_dynarr[i]) {
          unique_dynarr[i] inside {[21:30]};
      }
    };
    `SVTEST_CHECK(status == 1, "std::randomize scenario 4 failed")
    `SVTEST_CHECK(unique_queue.size() == 4, "Procedural array size corrupted")
    `SVTEST_CHECK(unique_queue[0] != unique_queue[1], "Uniqueness rule failed inside scope array")
    `SVTEST_CHECK(unique_queue[1] != unique_queue[2], "Uniqueness rule failed inside scope array")
    `SVTEST_CHECK(unique_queue[2] != unique_queue[3], "Uniqueness rule failed inside scope array")
      `SVTEST_CHECK(unique_dynarr.size() == 4, "Procedural darray size corrupted")
      `SVTEST_CHECK(unique_dynarr[0] != unique_dynarr[1], "Uniqueness rule failed inside scope darray")
      `SVTEST_CHECK(unique_dynarr[1] != unique_dynarr[2], "Uniqueness rule failed inside scope darray")
      `SVTEST_CHECK(unique_dynarr[2] != unique_dynarr[3], "Uniqueness rule failed inside scope darray")


    // -------------------------------------------------------------------------
    // Scenario 5: Packed Structures treated as Raw Integrals
    // -------------------------------------------------------------------------
    status = std::randomize(p_struct) with {
      p_struct == 8'hA5; // Driving memory slice as a raw 8-bit hex integral literal
    };
    `SVTEST_CHECK(status == 1, "std::randomize scenario 5 failed")
    `SVTEST_CHECK(p_struct.field_a == 4'hA && p_struct.field_b == 4'h5, 
                  "Packed struct breakdown failed to resolve raw integral configuration")


    // -------------------------------------------------------------------------
    // Scenario 6: 18.11.1 Scope Constraint Checker (std::randomize with no arguments)
    // -------------------------------------------------------------------------
    // When std::randomize() is called with an empty variable argument list, it 
    // treats all provided variables as constant values and strictly evaluates 
    // the conditional validation payload inside the 'with' block.
    
    div_num = 8'd50;
    div_den = 8'd10;

    // Passing condition: 50 / 10 is indeed 5
    status = std::randomize() with { div_num / div_den == 8'd5; };
    `SVTEST_CHECK(status == 1, "Scope checker rejected a mathematically valid equation state")

    // Failing condition: 50 / 10 is not 99
    status = std::randomize() with { div_num / div_den == 8'd99; };
    `SVTEST_CHECK(status == 0, "Scope checker failed to flag an invalid comparison state")

    // -------------------------------------------------------------------------
    // Report Final Matrix Results
    // -------------------------------------------------------------------------
    `SVTEST_PASSFAIL
  end

endmodule
