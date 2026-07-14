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

class ComprehensiveSystemFunctionBus;
  // Variables for Bit-Vector Vector Reduction Primitives
  rand bit [31:0] data_val;
  rand int        ones_count;
  rand bit [7:0]  onehot_val;
  rand bit [7:0]  onehot0_val;

  // Variables for Mathematical/Logarithmic System Functions
  rand bit [31:0] size_bytes;
  rand bit [4:0]  clog2_result;

  // Variables for Data Type Sign-Casting Primitives
  rand bit [7:0]  unsigned_byte;
  rand int        signed_sum;
  rand int        signed_sum_forced_to_unsigned;

  // Variables for Array Query Functions
  rand bit [7:0]  dyn_array[];
  rand int        array_size;

  // Variables for User-Defined Function validation (Scalar arguments)
  rand bit [15:0] seed_offset;
  rand bit [15:0] user_func_res;

  // New variables for User-Defined Function validation (Array arguments)
  rand bit [7:0]  payload_bytes[4]; 
  rand bit [7:0]  array_parity_res;

  // User-defined function 1: Scalar arguments
  static function bit [15:0] calculate_custom_hash(bit [31:0] data, bit [15:0] offset);
    calculate_custom_hash = (data[31:16] ^ data[15:0]) + offset;
  endfunction

  // User-defined function 2: Array argument passed into a constraint space.
  // Per the LRM, array arguments must be passed by value (no ref/output allowed)
  static function bit [7:0] calculate_array_parity(bit [7:0] arr[4]);
    bit [7:0] running_xor = 8'h00;
    foreach (arr[i]) begin
      running_xor ^= arr[i];
    end
    calculate_array_parity = running_xor;
  endfunction

  constraint valid_ranges {
    data_val   inside {[32'd1000 : 32'd50000]};
    size_bytes inside {[32'd1   : 32'd1024]};
    unsigned_byte == 8'hFF;
    seed_offset inside {[16'h1000 : 16'h2000]};
    
    // Set random data constraints on the input array elements
    foreach (payload_bytes[i]) {
      payload_bytes[i] inside {[8'h00 : 8'h7F]};
    }
  }

  // 1. Bit-Vector & Vector Profiling System Functions
  constraint bit_system_functions {
    ones_count  == $countones(data_val); 
    $onehot(onehot_val)   == 1'b1;       
    $onehot0(onehot0_val) == 1'b1;       
    onehot0_val == 8'h00; 
  }

  // 2. Mathematical Modeling System Functions
  constraint math_system_functions {
    clog2_result == $clog2(size_bytes);  
  }

  // 3. Structural Sign and Type Casting System Functions
  constraint casting_system_functions {
    signed_sum == $signed(unsigned_byte) + 32'sd1; 
    signed_sum_forced_to_unsigned == $signed(unsigned_byte) + 32'd1; 
  }

  // 4. Structural Array/Collection Information Queries
  constraint array_system_functions {
    array_size == $size(dyn_array); 
  }

  // 5. User-Defined Function Constraint Blocks (Scalar and Array types)
  constraint user_defined_functions {
    user_func_res    == calculate_custom_hash(data_val, seed_offset);
    array_parity_res == calculate_array_parity(payload_bytes);
  }

  // Post-randomize loop to allocate size dynamically before query testing
  function void pre_randomize();
    dyn_array = new[10]; // Explicitly size to 10 elements
  endfunction
endclass

// =============================================================================
// Testbench Module Architecture
// =============================================================================
module tb_top;

  initial begin
    `SVTEST_INIT

    ComprehensiveSystemFunctionBus bus = new();
    int status;

    repeat (20) begin
      status = bus.randomize();
      `SVTEST_CHECK(status == 1, "Randomization failed on function matrices")

      // Verify Category 1: Bit Vectors
      `SVTEST_CHECK(bus.ones_count == $countones(bus.data_val), "Mismatch on $countones execution")
      `SVTEST_CHECK($onehot(bus.onehot_val) == 1'b1, "Mismatch on $onehot assertion execution")
      `SVTEST_CHECK($onehot0(bus.onehot0_val) == 1'b1, "Mismatch on $onehot0 assertion execution")

      // Verify Category 2: Mathematical Scaling
      `SVTEST_CHECK(bus.clog2_result == $clog2(bus.size_bytes), "Mismatch on $clog2 logarithmic boundary")

      // Verify Category 3: Explicit Casting Sign Conversions
      `SVTEST_CHECK(bus.signed_sum == 32'd0, $sformatf("Mismatch on $signed context resolution casting math : unsigned : %0d, signed sum : %0d",bus.unsigned_byte,bus.signed_sum))
      `SVTEST_CHECK(bus.signed_sum_forced_to_unsigned == 32'd256, "Mismatch on unsigned forcing - use of any unsigned quantity in the expression should force unsigned casting for every variable in the expression")

      // Verify Category 4: Structural Array Dimension Queries
      `SVTEST_CHECK(bus.array_size == 10, "Mismatch on dynamic $size element evaluation check")

      // Verify Category 5: User-Defined Function Resolution (Scalar Arguments)
      `SVTEST_CHECK(bus.user_func_res == bus.calculate_custom_hash(bus.data_val, bus.seed_offset), 
                    "Mismatch on user-defined function math resolution inside constraint")

      // Verify Category 6: User-Defined Function Resolution (Array Arguments)
      `SVTEST_CHECK(bus.array_parity_res == bus.calculate_array_parity(bus.payload_bytes),
                    "Mismatch on user-defined array processing function result inside constraint")
    end

    `SVTEST_PASSFAIL
  end

endmodule
