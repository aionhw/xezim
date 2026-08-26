`timescale 1ns/1ps

`include "../common/svtest_defs.svh"

// Test: specparam hierarchical access (IEEE 1800-2023 §6.20.5)
// Validates specparam/localparam hierarchical access across module instances
// and specparam usage in specify blocks (§6.20.5, §6.20.6, §6.20.7)
// Also tests specparam access in generate blocks (§27)

// Child module with specparam that depends on parameter
module child #(
  parameter int CHILD_DELAY = 2
);
  specparam SPEC_DELAY = CHILD_DELAY + 1;
  localparam int LOCAL_DELAY = CHILD_DELAY + 1;

  // Ports for specify block timing tests
  input logic clk;
  input logic d;
  output logic q;
  assign q = d;  // Simple flip-flop model

  // Specify block using specparam for timing checks (§6.20.6, §6.20.7)
  specify
    (clk => q) = SPEC_DELAY;
    $setup(d, clk, SPEC_DELAY);
    $hold(clk, d, SPEC_DELAY);
    $width(posedge clk, SPEC_DELAY);
  endspecify
endmodule

// Intermediate module with its own specparam
module intermediate #(
  parameter int MID_DELAY = 3
);
  specparam MID_SPEC = MID_DELAY + 1;
  localparam int MID_LOCAL = MID_DELAY + 1;

  child #(.CHILD_DELAY(MID_DELAY)) u_child();
endmodule

// Parent uses child's specparam and localparam in its own parameters
module top;
  `SVTEST_INIT

  // Test 1: Direct child instance
  child #(.CHILD_DELAY(5)) u_child();

  // Test 2: Intermediate with its own specparam
  intermediate #(.MID_DELAY(7)) u_intermediate();

  // Test 3: Child with parameter expression (not just literal)
  parameter int BASE_DELAY = 4;
  child #(.CHILD_DELAY(BASE_DELAY + 3)) u_child_expr();  // CHILD_DELAY = 7

  // Test 4: Generate block with specparam access
  genvar g;
  generate
    for (g = 0; g < 3; g++) begin : gen_child
      child #(.CHILD_DELAY(g + 2)) u_gen_child();
    end
  endgenerate

  // Check in initial block (runtime access)
  initial begin
    // Level 1: Direct child instance
    `SVTEST_CHECK(u_child.SPEC_DELAY == 6, "child.SPEC_DELAY = 6 (5+1)")
    `SVTEST_CHECK(u_child.LOCAL_DELAY == 6, "child.LOCAL_DELAY = 6 (5+1)")

    // Level 2: Intermediate's own specparam
    `SVTEST_CHECK(u_intermediate.MID_SPEC == 8, "intermediate.MID_SPEC = 8 (7+1)")
    `SVTEST_CHECK(u_intermediate.MID_LOCAL == 8, "intermediate.MID_LOCAL = 8 (7+1)")

    // Level 2: Child's specparam through intermediate (runtime access)
    `SVTEST_CHECK(u_intermediate.u_child.SPEC_DELAY == 8, "intermediate.child.SPEC_DELAY = 8")
    `SVTEST_CHECK(u_intermediate.u_child.LOCAL_DELAY == 8, "intermediate.child.LOCAL_DELAY = 8")

    // Level 3: Child with parameter expression
    `SVTEST_CHECK(u_child_expr.SPEC_DELAY == 8, "child_expr.SPEC_DELAY = 8 (7+1)")
    `SVTEST_CHECK(u_child_expr.LOCAL_DELAY == 8, "child_expr.LOCAL_DELAY = 8 (7+1)")

    // Generate block: runtime hierarchical access to generated instances
    // Note: localparam in generate scope doesn't capture elaboration-time values
    // but runtime access via hierarchical paths works correctly
    `SVTEST_CHECK(gen_child[0].u_gen_child.SPEC_DELAY == 3, "gen_child[0].SPEC_DELAY = 3 (2+1)")
    `SVTEST_CHECK(gen_child[1].u_gen_child.SPEC_DELAY == 4, "gen_child[1].SPEC_DELAY = 4 (3+1)")
    `SVTEST_CHECK(gen_child[2].u_gen_child.SPEC_DELAY == 5, "gen_child[2].SPEC_DELAY = 5 (4+1)")
    `SVTEST_CHECK(gen_child[0].u_gen_child.LOCAL_DELAY == 3, "gen_child[0].LOCAL_DELAY = 3 (2+1)")
    `SVTEST_CHECK(gen_child[1].u_gen_child.LOCAL_DELAY == 4, "gen_child[1].LOCAL_DELAY = 4 (3+1)")
    `SVTEST_CHECK(gen_child[2].u_gen_child.LOCAL_DELAY == 5, "gen_child[2].LOCAL_DELAY = 5 (4+1)")

    `SVTEST_PASSFAIL
  end
endmodule