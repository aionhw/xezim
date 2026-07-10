module vpi_backdoor_compliance;
  // Structures
  typedef struct packed {
    logic [15:0] field_a;
    logic [15:0] field_b;
  } packed_struct_t;

  typedef struct {
    int  val_i;
    real val_r;
  } unpacked_struct_t;

  // All SV types to test
  byte               t_byte;
  shortint           t_shortint;
  int                t_int;
  longint            t_longint;
  time               t_time;
  real               t_real;
  shortreal          t_shortreal;
  bit [127:0]        t_vec128;
  bit [127:0]        read_vec128;
  wire [31:0]        t_net32;
  reg [31:0]         t_net32_driver;
  
  packed_struct_t    t_packed_struct;
  unpacked_struct_t  t_unpacked_struct;
  
  int                errors;
  int                temp_val;

  assign t_net32 = t_net32_driver;

  // DPI context imports for integer-like types
  import "DPI-C" context function int backdoor_read_int(input string path, output int val);
  import "DPI-C" context function int backdoor_force_int(input string path, input int val);

  // DPI context imports for reals
  import "DPI-C" context function int backdoor_read_real(input string path, output real val);
  import "DPI-C" context function int backdoor_force_real(input string path, input real val);

  // DPI context imports for vectors
  import "DPI-C" context function int backdoor_read_vec128(input string path, output bit [127:0] val);
  import "DPI-C" context function int backdoor_force_vec128(input string path, input bit [127:0] val);

  // Global release API
  import "DPI-C" context function int backdoor_release(input string path);

  // Verification helper macros
  `define assert_eq(val, exp, msg) \
    if ((val) !== (exp)) begin \
      $display("FAIL: %s. Got %p, expected %p", msg, val, exp); \
      errors = errors + 1; \
    end

  initial begin
    errors = 0;
    $display("=== SystemVerilog VPI/DPI Backdoor Comprehensive Type Test ===");

    // ----------------------------------------------------
    // TYPE: int
    // ----------------------------------------------------
    t_int = 32'h00FF_00FF;
    #1;
    `assert_eq(t_int, 32'h00FF_00FF, "int init")

    if (!backdoor_force_int("vpi_backdoor_compliance.t_int", 32'hCAFE_BABE)) errors = errors + 1;
    #1;
    `assert_eq(t_int, 32'hCAFE_BABE, "int force")

    t_int = 32'hBEEF_DEAD;
    #1;
    `assert_eq(t_int, 32'hCAFE_BABE, "int assign blocked during force")

    // Read the variable value via VPI backdoor
    if (!backdoor_read_int("vpi_backdoor_compliance.t_int", temp_val)) begin
      $display("T1.0 FAIL: Read call failed");
      errors = errors + 1;
    end
    `assert_eq(temp_val, 32'hCAFE_BABE, "int read forced value")

    if (!backdoor_release("vpi_backdoor_compliance.t_int")) errors = errors + 1;
    #1;
    `assert_eq(t_int, 32'hCAFE_BABE, "int release retain value")

    t_int = 32'hBEEF_DEAD;
    #1;
    `assert_eq(t_int, 32'hBEEF_DEAD, "int post-release assign")

    // ----------------------------------------------------
    // TYPE: byte (mapped via int APIs)
    // ----------------------------------------------------
    t_byte = 8'h0F;
    #1;
    if (!backdoor_force_int("vpi_backdoor_compliance.t_byte", 8'hF0)) errors = errors + 1;
    #1;
    `assert_eq(t_byte, 8'hF0, "byte force")

    if (!backdoor_release("vpi_backdoor_compliance.t_byte")) errors = errors + 1;
    #1;

    // ----------------------------------------------------
    // TYPE: real
    // ----------------------------------------------------
    t_real = 1.25;
    #1;
    if (!backdoor_force_real("vpi_backdoor_compliance.t_real", 9.81)) errors = errors + 1;
    #1;
    `assert_eq(t_real, 9.81, "real force")

    t_real = 0.0;
    #1;
    `assert_eq(t_real, 9.81, "real assign blocked during force")

    if (!backdoor_release("vpi_backdoor_compliance.t_real")) errors = errors + 1;
    #1;
    `assert_eq(t_real, 9.81, "real release retain value")

    // ----------------------------------------------------
    // TYPE: shortreal (mapped via real APIs)
    // ----------------------------------------------------
    t_shortreal = 2.5;
    #1;
    if (!backdoor_force_real("vpi_backdoor_compliance.t_shortreal", 4.5)) errors = errors + 1;
    #1;
    `assert_eq(t_shortreal, 4.5, "shortreal force")

    if (!backdoor_release("vpi_backdoor_compliance.t_shortreal")) errors = errors + 1;
    #1;

    // ----------------------------------------------------
    // TYPE: bit [127:0] (Vector type)
    // ----------------------------------------------------
    t_vec128 = 128'h0123456789ABCDEF_FEDCBA9876543210;
    #1;
    // Read it back through vpiVectorVal. This was imported but never
    // called, which is why vpi_get_value silently ignoring vpiVectorVal
    // went unnoticed — the very format UVM's HDL backdoor reads with.
    read_vec128 = 128'hDEAD_BEEF;   // poison, so a no-op read is visible
    if (!backdoor_read_vec128("vpi_backdoor_compliance.t_vec128", read_vec128)) errors = errors + 1;
    `assert_eq(read_vec128, 128'h0123456789ABCDEF_FEDCBA9876543210, "vec128 read")

    if (!backdoor_force_vec128("vpi_backdoor_compliance.t_vec128", 128'h5A5A5A5A5A5A5A5A_A5A5A5A5A5A5A5A5)) errors = errors + 1;
    #1;
    if (!backdoor_read_vec128("vpi_backdoor_compliance.t_vec128", read_vec128)) errors = errors + 1;
    `assert_eq(read_vec128, 128'h5A5A5A5A5A5A5A5A_A5A5A5A5A5A5A5A5, "vec128 read after force")
    `assert_eq(t_vec128, 128'h5A5A5A5A5A5A5A5A_A5A5A5A5A5A5A5A5, "vec128 force")

    t_vec128 = 128'h0;
    #1;
    `assert_eq(t_vec128, 128'h5A5A5A5A5A5A5A5A_A5A5A5A5A5A5A5A5, "vec128 assign blocked during force")

    if (!backdoor_release("vpi_backdoor_compliance.t_vec128")) errors = errors + 1;
    #1;
    `assert_eq(t_vec128, 128'h5A5A5A5A5A5A5A5A_A5A5A5A5A5A5A5A5, "vec128 release retain value")

    // ----------------------------------------------------
    // TYPE: wire [31:0] (Net type)
    // ----------------------------------------------------
    t_net32_driver = 32'hA5A5_5A5A;
    #1;
    `assert_eq(t_net32, 32'hA5A5_5A5A, "net init driver")

    if (!backdoor_force_int("vpi_backdoor_compliance.t_net32", 32'hFFFF_FFFF)) errors = errors + 1;
    #1;
    `assert_eq(t_net32, 32'hFFFF_FFFF, "net force")

    t_net32_driver = 32'h0000_0000;
    #1;
    `assert_eq(t_net32, 32'hFFFF_FFFF, "net driver change ignored during force")

    if (!backdoor_release("vpi_backdoor_compliance.t_net32")) errors = errors + 1;
    #1;
    `assert_eq(t_net32, 32'h0000_0000, "net release restores continuous driver")

    // ----------------------------------------------------
    // TYPE: packed struct (32-bit vector mapping)
    // ----------------------------------------------------
    t_packed_struct.field_a = 16'hAAAA;
    t_packed_struct.field_b = 16'h5555;
    #1;
    `assert_eq(t_packed_struct, {16'hAAAA, 16'h5555}, "packed struct init")

    if (!backdoor_force_int("vpi_backdoor_compliance.t_packed_struct", 32'h1234_5678)) errors = errors + 1;
    #1;
    `assert_eq(t_packed_struct, {16'h1234, 16'h5678}, "packed struct force")

    t_packed_struct = {16'h0000, 16'h0000};
    #1;
    `assert_eq(t_packed_struct, {16'h1234, 16'h5678}, "packed struct assign blocked")

    if (!backdoor_release("vpi_backdoor_compliance.t_packed_struct")) errors = errors + 1;
    #1;
    `assert_eq(t_packed_struct, {16'h1234, 16'h5678}, "packed struct release retain")

    // ----------------------------------------------------
    // TYPE: unpacked struct (tested via member hierarchical paths)
    // ----------------------------------------------------
    t_unpacked_struct.val_i = 10;
    t_unpacked_struct.val_r = 1.5;
    #1;
    `assert_eq(t_unpacked_struct.val_i, 10, "unpacked struct member val_i init")
    `assert_eq(t_unpacked_struct.val_r, 1.5, "unpacked struct member val_r init")

    if (!backdoor_force_int("vpi_backdoor_compliance.t_unpacked_struct.val_i", 42)) errors = errors + 1;
    if (!backdoor_force_real("vpi_backdoor_compliance.t_unpacked_struct.val_r", 5.5)) errors = errors + 1;
    #1;
    `assert_eq(t_unpacked_struct.val_i, 42, "unpacked struct member val_i force")
    `assert_eq(t_unpacked_struct.val_r, 5.5, "unpacked struct member val_r force")

    t_unpacked_struct.val_i = 0;
    t_unpacked_struct.val_r = 0.0;
    #1;
    `assert_eq(t_unpacked_struct.val_i, 42, "unpacked struct member val_i assign blocked")
    `assert_eq(t_unpacked_struct.val_r, 5.5, "unpacked struct member val_r assign blocked")

    if (!backdoor_release("vpi_backdoor_compliance.t_unpacked_struct.val_i")) errors = errors + 1;
    if (!backdoor_release("vpi_backdoor_compliance.t_unpacked_struct.val_r")) errors = errors + 1;
    #1;
    `assert_eq(t_unpacked_struct.val_i, 42, "unpacked struct member val_i release retain")
    `assert_eq(t_unpacked_struct.val_r, 5.5, "unpacked struct member val_r release retain")

    t_unpacked_struct.val_i = 99;
    t_unpacked_struct.val_r = 9.9;
    #1;
    `assert_eq(t_unpacked_struct.val_i, 99, "unpacked struct member val_i post-release assign")
    `assert_eq(t_unpacked_struct.val_r, 9.9, "unpacked struct member val_r post-release assign")

    // ----------------------------------------------------
    // FINAL REPORT
    // ----------------------------------------------------
    if (errors == 0) begin
      $display("RESULT: PASSED");
    end else begin
      $display("RESULT: FAILED with %d errors", errors);
    end
    $finish;
  end
endmodule
