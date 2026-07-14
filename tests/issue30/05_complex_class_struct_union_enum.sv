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
// Helper Types & Sub-Class Structures
// =============================================================================
typedef enum bit [1:0] { VAL_A = 2'b00, VAL_B = 2'b01 } legal_enum_e;

// Sub-class to be instantiated and randomized via an object handle
class LeafObject;
  rand bit [7:0] leaf_val;
  constraint leaf_rule { leaf_val > 8'd50; }
endclass

// Packed structure treated as an integral type.
// It contains a nested enum type to test the LRM enum boundary override rule.
typedef struct packed {
  legal_enum_e  nested_enum; // Normally limited to 2'b00 or 2'b01
  bit [5:0]     payload;
} packed_struct_s;

// Packed untagged union treated as a standard raw integral type
typedef union packed {
  legal_enum_e  union_enum;  // Shared memory footprint
  bit [1:0]     raw_bits;
} packed_union_u;

// Unpacked structure containing individual random and cyclic members
typedef struct {
  rand  bit [7:0]   unpacked_rand_val;
  randc bit [1:0]   unpacked_randc_val; 
} unpacked_struct_s;


// =============================================================================
// Primary LRM Test Container Class
// =============================================================================
class ConcurrentStructureBus;

  // Rule 1: Object handles declared rand are solved concurrently.
  // Object handles cannot be declared randc.
  rand LeafObject leaf_inst;

  // Rule 2: Unpacked structures solved concurrently member-by-member.
  // The structure itself cannot be randc, but members can be rand or randc.
  rand unpacked_struct_s unpacked_struct_inst;

  // Rule 3 & 4: Packed structures and untagged unions are treated as raw integral types.
  // They can be declared rand or randc.
  rand packed_struct_s packed_struct_inst;
  rand packed_union_u  packed_union_inst;

  // Variables to hold handle targets for tracking references
  LeafObject historical_handle;

  // NEW: Explicit class constructor to securely instantiate child objects
  function new();
    leaf_inst = new(); 
  endfunction
  
  // Constraints proving concurrent evaluation between top-level and sub-structures
  constraint structural_cross_rules {
    // Cross-constraint matching top-level packed structure with nested object handle member
    leaf_inst.leaf_val == packed_struct_inst.payload + 8'd10;
    
    // Rule 5: Force the nested enums to land on values OUTSIDE their legal enum definitions.
    // LRM says: "rules in 18.3 restricting the random values of an enum variable shall not apply to that member"
    // Because they reside in a packed structure/union, values 2'b10 and 2'b11 are legal targets.
    packed_struct_inst.nested_enum == 2'b11; 
    packed_union_inst.union_enum   == 2'b10;
  }

endclass


// =============================================================================
// Testbench Module Architecture
// =============================================================================
module tb_top;

  initial begin
    `SVTEST_INIT
    int status ;

    ConcurrentStructureBus bus = new();
        
    // Store original memory footprint reference to ensure handle doesn't shift
    bus.historical_handle = bus.leaf_inst;

    repeat (20) begin
      status = bus.randomize();
      `SVTEST_CHECK(status == 1, "Solver failed concurrent structural randomization matrices")

      // -----------------------------------------------------------------------
      // Self-Checking Verification Loops
      // -----------------------------------------------------------------------

      // Verify Rule 1: Handle pointer must remain identical (Randomization does not modify handle)
      `SVTEST_CHECK(bus.leaf_inst == bus.historical_handle, 
                    "LRM Violation: Object handle pointer variable changed during randomization")

      // Verify Rule 1: Concurrent evaluation checked sub-class constraints successfully
      `SVTEST_CHECK(bus.leaf_inst.leaf_val > 8'd50, 
                    "Concurrent object handle constraint evaluation failed")

      // Verify Rule 2: Unpacked structure internal variables solved correctly
      `SVTEST_CHECK(bus.unpacked_struct_inst.unpacked_randc_val inside {[0:3]}, 
                    "Unpacked structure nested member randc out of range bounds")

      // Verify Rule 3, 4 & 5: Packed structures/unions treated as raw integral values.
      // Confirm cross-constraint concurrent equation resolved cleanly across variables
      `SVTEST_CHECK(bus.leaf_inst.leaf_val == bus.packed_struct_inst.payload + 8'd10,
                    "Cross-object structural equation failed resolution")

      // Verify Rule 5: Confirm nested enum constraints bypassed normal enum range limits
      // This checks if the tool allowed the invalid enum raw bits (2'b11 and 2'b10) to be driven
      `SVTEST_CHECK(bus.packed_struct_inst.nested_enum == 2'b11,
                    "Packed struct nested enum rule failed: 2'b11 was suppressed or blocked")
      `SVTEST_CHECK(bus.packed_union_inst.union_enum == 2'b10,
                    "Packed untagged union nested enum rule failed: 2'b10 was suppressed or blocked")
    end

    // -----------------------------------------------------------------------
    // Report Test Results
    // -----------------------------------------------------------------------
    `SVTEST_PASSFAIL
  end

endmodule
