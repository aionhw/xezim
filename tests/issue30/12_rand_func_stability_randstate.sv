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

  initial begin
    `SVTEST_INIT

    // State capturing variables
    string rng_checkpoint;
    int    val;
    bit    min_hit = 0;
    bit    max_hit = 0;

    // Data structures for tracking shuffle stability
    int shuffle_array[] = '{10, 20, 30, 40, 50};
    int gold_shuffled[];

    // Data structures for tracking randcase stability
    int gold_randcase_choice;
    int test_randcase_choice;

    // Data structures for tracking randsequence stability
    int gold_sequence_trail[$];
    int test_sequence_trail[$];
    
    process p ;
    
    p = process::self();

    // Initialize the primary thread seed to ensure a predictable baseline
    void'($urandom(32'hFEED_BEEF));


    // -------------------------------------------------------------------------
    // Test 1: $urandom_range Boundary Inclusivity & Argument Independence
    // -------------------------------------------------------------------------
    repeat (100) begin
      // LRM 18.13: If min > max, arguments are automatically reversed. 
      // This test uses (min=5, max=15) but provides them as (15, 5)
      val = $urandom_range(15, 5);
      `SVTEST_CHECK(val >= 5 && val <= 15, "$urandom_range generated value out of reversed bounds")
      if (val == 5)  min_hit = 1;
      if (val == 15) max_hit = 1;
    end
    // Ensure boundaries are inclusive
    `SVTEST_CHECK(min_hit && max_hit, "$urandom_range limits were not inclusive over 100 iterations")


    // -------------------------------------------------------------------------
    // Test 2: Random Stability of the .shuffle() Method
    // -------------------------------------------------------------------------
    // Capture state immediately before shuffling
    rng_checkpoint = p.get_randstate();
    
    shuffle_array.shuffle();
    gold_shuffled = shuffle_array; // Save the golden shuffled result

    // Restore state and reshuffle. The output array layout must be identical.
    p.set_randstate(rng_checkpoint);
    shuffle_array = '{10, 20, 30, 40, 50}; // Reset to pre-shuffle baseline
    shuffle_array.shuffle();

    foreach (shuffle_array[i]) begin
      `SVTEST_CHECK(shuffle_array[i] == gold_shuffled[i], "shuffle() stability broken: rollback yielded different order")
    end


    // -------------------------------------------------------------------------
    // Test 3: Random Stability of randcase Control Blocks
    // -------------------------------------------------------------------------
    // Advance thread slightly and capture state
    void'($urandom());
    rng_checkpoint = p.get_randstate();

    // First execution pass
    randcase
      30 : gold_randcase_choice = 1;
      50 : gold_randcase_choice = 2;
      20 : gold_randcase_choice = 3;
    endcase

    // Restore state and execute again. The exact same branch must be selected.
    p.set_randstate(rng_checkpoint);
    randcase
      30 : test_randcase_choice = 1;
      50 : test_randcase_choice = 2;
      20 : test_randcase_choice = 3;
    endcase

    `SVTEST_CHECK(test_randcase_choice == gold_randcase_choice, "randcase stability broken: branch choice diverged")


    // -------------------------------------------------------------------------
    // Test 4: Random Stability of randsequence Structural Grammar
    // -------------------------------------------------------------------------
    // Capture state prior to driving production rules
    rng_checkpoint = p.get_randstate();

    // First execution pass
    randsequence( main_stream )
      main_stream : node_a node_b;
      node_a      : choice_1 := 40 | choice_2 := 60;
      node_b      : choice_3 | choice_4;

      choice_1    : { gold_sequence_trail.push_back(1); };
      choice_2    : { gold_sequence_trail.push_back(2); };
      choice_3    : { gold_sequence_trail.push_back(3); };
      choice_4    : { gold_sequence_trail.push_back(4); };
    endsequence

    // Restore state and re-execute production rules
    p.set_randstate(rng_checkpoint);
    randsequence( main_stream )
      main_stream : node_a node_b;
      node_a      : choice_1 := 40 | choice_2 := 60;
      node_b      : choice_3 | choice_4;

      choice_1    : { test_sequence_trail.push_back(1); };
      choice_2    : { test_sequence_trail.push_back(2); };
      choice_3    : { test_sequence_trail.push_back(3); };
      choice_4    : { test_sequence_trail.push_back(4); };
    endsequence

    // Verify that the generated stream exactly matches the first pass item-by-item
    `SVTEST_CHECK(test_sequence_trail.size() == gold_sequence_trail.size(), "randsequence trail length mismatch")
    foreach (test_sequence_trail[k]) begin
      `SVTEST_CHECK(test_sequence_trail[k] == gold_sequence_trail[k], "randsequence stability broken: decision branch diverged")
    end


    // -------------------------------------------------------------------------
    // Report Final Results
    // -------------------------------------------------------------------------
    `SVTEST_PASSFAIL
  end

endmodule
