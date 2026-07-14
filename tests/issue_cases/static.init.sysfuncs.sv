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

module sv_static_init_tb;

  // 1. Math / Dimension Functions (Elaboration-safe)
  static int         math_clog2 = $clog2(32);
  static real        math_sqrt  = $sqrt(16.0);
  static int         array_size = $size(logic [7:0]);

  // 2. String & Hierarchy Functions (Simulation-safe)
  // Note: $sformatf("%m") captures the exact instance path at time zero


  static string      inst_path  = $sformatf("Path is %m");
  static string      type_name  = $typename(int);

  // 3. Command Line / Environment Functions (Simulation-safe)
  // To test plusargs, run simulation with: +TEST_MODE=1 +SEED_VAL=42
  static int         has_plusarg = $test$plusargs("TEST_MODE");
  static int         plusarg_val = get_plusarg_seed();

  // Helper function to extract a plusarg into a static variable at initialization
  function static int get_plusarg_seed();
    int val;
    if ($value$plusargs("SEED_VAL=%d", val)) begin
      return val;
    end
    return 0; // Default if not found
  endfunction

  // 4. Randomization Functions (Simulation-safe)
  static int         rand_init  = $urandom_range(100, 200);

  // -------------------------------------------------------------
  // Test Execution Block
  // -------------------------------------------------------------
  initial begin
    // Instantiate your macro to create the 'failures' counter
    `SVTEST_INIT

    $display("--- Starting Static Initialization Verification ---");

    // Check Math and Dimension initializations
    `SVTEST_CHECK(math_clog2 == 5,    "math_clog2 should be 5")
    `SVTEST_CHECK(math_sqrt == 4.0,   "math_sqrt should be 4.0")
    `SVTEST_CHECK(array_size == 8,    "array_size should be 8")

    // Check String and Hierarchy initializations
    `SVTEST_CHECK(inst_path == "Path is sv_static_init_tb", "inst_path mismatch")
    `SVTEST_CHECK(type_name == "int", "type_name mismatch")

    // Check Environment / Plusarg initializations
    // (Assumes +TEST_MODE and +SEED_VAL=42 are passed to simulation)
    `SVTEST_CHECK(has_plusarg == 1,   "has_plusarg should be 1 (Run with +TEST_MODE)")
    `SVTEST_CHECK(plusarg_val == 42,  "plusarg_val should be 42 (Run with +SEED_VAL=42)")

    // Check Randomization boundaries
    `SVTEST_CHECK(rand_init >= 100 && rand_init <= 200, "rand_init out of range [100,200]")

    // Print final report and exit
    `SVTEST_PASSFAIL
  end

endmodule
