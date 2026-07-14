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
// Helper Target Class 
// =============================================================================
class StableObject;
  rand bit [31:0] obj_rand_data;
  
  function void seed_internally(int unsigned custom_seed);
    // 'this.srandom()' initializes this specific object context's private RNG
    this.srandom(custom_seed);
  endfunction
endclass

// =============================================================================
// Testbench Module Architecture
// =============================================================================
module tb_top;

  initial begin
    `SVTEST_INIT

    StableObject my_obj = new();
    
    // Dynamic seed inputs derived from variables rather than hardcoded literals
    static int unsigned dynamic_thread_seed = 32'hAAAA_BBBB;
    static int unsigned dynamic_object_seed = 32'hCCCC_DDDD;

    // Capture storage for reproducibility verification
    int unsigned thread_gold_1, thread_gold_2, thread_gold_3;
    int unsigned object_gold_1 ;
    
    int unsigned fork_t1_val, fork_t2_val;

    // -------------------------------------------------------------------------
    // Test 1: The Difference: Class Object srandom() vs Process Thread srandom()
    // -------------------------------------------------------------------------
    
    // Step 1A: Seed the active execution THREAD process via process::self()
    // This alters the output stream of standalone system functions like $urandom()
    begin
      static process p = process::self();
      p.srandom(dynamic_thread_seed);
    end
    
    // Step 1B: Seed the CLASS OBJECT using its internal method
    // This alters the output stream of the class solver engine (.randomize())
    my_obj.seed_internally(dynamic_object_seed);

    // Capture the initial golden baseline values
    thread_gold_1 = $urandom();
    void'(my_obj.randomize());
    object_gold_1 = my_obj.obj_rand_data;

    thread_gold_2 = $urandom(); // thread step 2
    thread_gold_3 = $urandom(); // thread step 3

    // --- RE-SEED AND VALIDATE ISOLATION ---
    
    // Scenario 1: Only reset the Thread process seed. Object stream must NOT change.
    begin
      static process p = process::self();
      p.srandom(dynamic_thread_seed);
    end
    
    `SVTEST_CHECK($urandom() == thread_gold_1, "Process srandom failed to reproduce thread step 1")
     // This value would normally match object_gold_1 if object was affected. 
    // It should be completely different because we didn't re-seed the object.
    void'(my_obj.randomize());
    `SVTEST_CHECK(my_obj.obj_rand_data != object_gold_1, "Object RNG was unexpectedly reset by a process seed update")

     // Scenario 2: Reset the Object seed. Thread stream must NOT be disturbed.
    // The thread is currently sitting at Step 2 of its current sequence.
    my_obj.seed_internally(dynamic_object_seed);
    
    // Check that the thread stream continues exactly where it left off (Step 2 and Step 3)
    // proving the object seed update had 0 impact on the thread's sequence pointer.
    `SVTEST_CHECK($urandom() == thread_gold_2, "Thread RNG sequence was corrupted or shifted by an object seed update (Step 2 check)")
    `SVTEST_CHECK($urandom() == thread_gold_3, "Thread RNG sequence was corrupted or shifted by an object seed update (Step 3 check)")

    // Verify the object successfully rolled back to its original Step 1 value
    void'(my_obj.randomize());
    `SVTEST_CHECK(my_obj.obj_rand_data == object_gold_1, "Object srandom failed to reproduce object step 1")

    // -------------------------------------------------------------------------
    // Test 2: Forked Parallel Threads & Variable Seeding Stability
    // -------------------------------------------------------------------------
    // Under Section 18.14.1, when parallel threads are forked, they automatically
    // inherit independent, isolated random state definitions derived from the parent.
    // Manually updating a thread seed using a variable guarantees that its random 
    // generation sequence is entirely decoupled from execution order variations.
    
    fork
      // Thread 1
      begin
        static process p1 = process::self();
        p1.srandom(dynamic_thread_seed + 1); // Pass a dynamic variable calculation
        #10; // Deliberate scheduling delay
        fork_t1_val = $urandom();
      end

      // Thread 2
      begin
        static process p2 = process::self();
        p2.srandom(dynamic_thread_seed + 2); // Isolated seed context
        #5;  // Shorter scheduling delay; executes prior to Thread 1
        fork_t2_val = $urandom();
      end
    join

    // --- VERIFY REPRODUCIBILITY DESPITE DELAY ORDER INVERSION ---
    // We re-execute the threads with identical variable seeds but swap the execution delays.
    // If thread stability holds true, the values generated MUST remain completely unchanged.
    
    fork
      // Thread 1 (Now executing with NO delay, hitting the RNG first)
      begin
        static process p1 = process::self();
        p1.srandom(dynamic_thread_seed + 1);
        `SVTEST_CHECK($urandom() == fork_t1_val, "Thread 1 random stability broken by scheduling modifications")
      end

      // Thread 2 (Now execution is delayed)
      begin
        static process p2 = process::self();
        p2.srandom(dynamic_thread_seed + 2);
        #20; 
        `SVTEST_CHECK($urandom() == fork_t2_val, "Thread 2 random stability broken by scheduling modifications")
      end
    join

    // -------------------------------------------------------------------------
    // Report Final Results
    // -------------------------------------------------------------------------
    `SVTEST_PASSFAIL
  end

endmodule
