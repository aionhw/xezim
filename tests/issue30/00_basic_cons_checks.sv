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

// Class demonstrating concepts defined in Section 18.3 (Concepts and Usage)
class Section183Bus;
  // 'rand' modifier specifies standard uniform random variables
  rand bit [15:0] addr;
  rand bit [31:0] data;
  
  // 'randc' modifier specifies cyclic random variables
  // It cycles through all values in permutation before repeating
  randc bit [1:0] cyclic_id;

  // Constraint block to limit solver's solution space
  constraint word_align {
    addr[1:0] == 2'b0; 
  }
  
  constraint data_payload_bounds {
    data >= 32'h1000_0000;
    data <= 32'h2000_0000;
  }
endclass

module tb_top;

  initial begin
    `SVTEST_INIT

    Section183Bus bus = new();
    int status;
    bit seen_ids[4];
    
    // -------------------------------------------------------------------------
    // Test 1: Verify cyclic properties of a randc variable
    // -------------------------------------------------------------------------
    // Reset track buffer
    for (int i = 0; i < 4; i++) begin
      seen_ids[i] = 0;
    end

    // Over 4 iterations, a 2-bit randc variable must visit every element 
    // (0, 1, 2, and 3) exactly once before repeating any value.
    // Ensure this is the first set of randomizations
    repeat (4) begin
      status = bus.randomize();
      `SVTEST_CHECK(status == 1, "randomize() failed during cyclic testing")
      seen_ids[bus.cyclic_id] = 1;
    end

    // Validate that all permutations were hit during the cycle
    `SVTEST_CHECK(seen_ids[0] == 1, "Cyclic ID 0 was missed in the permutation cycle")
    `SVTEST_CHECK(seen_ids[1] == 1, "Cyclic ID 1 was missed in the permutation cycle")
    `SVTEST_CHECK(seen_ids[2] == 1, "Cyclic ID 2 was missed in the permutation cycle")
    `SVTEST_CHECK(seen_ids[3] == 1, "Cyclic ID 3 was missed in the permutation cycle")

    // -------------------------------------------------------------------------
    // Test 2: Verify rand allocation and standard constraint satisfaction
    // -------------------------------------------------------------------------
    repeat (50) begin
      // .randomize() returns 1 on success and 0 on failure
      status = bus.randomize();
      `SVTEST_CHECK(status == 1, "randomize() execution failed")
      
      // Check word alignment constraint enforcement (addr[1:0] == 0)
      `SVTEST_CHECK(bus.addr[1:0] == 2'b0, "Address is not word-aligned")
      
      // Check range bounds constraint enforcement
      `SVTEST_CHECK(bus.data >= 32'h1000_0000 && bus.data <= 32'h2000_0000, 
                    "Data payload is outside constrained bounds")
    end


    // -------------------------------------------------------------------------
    // Report Final Results
    // -------------------------------------------------------------------------
    `SVTEST_PASSFAIL
  end

endmodule
