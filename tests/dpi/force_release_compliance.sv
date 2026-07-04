module force_release_compliance;
  // Structures
  typedef struct packed {
    logic [15:0] field_a;
    logic [15:0] field_b;
  } packed_struct_t;

  // All SV types to test
  byte               t_byte;
  shortint           t_shortint;
  int                t_int;
  longint            t_longint;
  time               t_time;
  real               t_real;
  shortreal          t_shortreal;
  bit [127:0]        t_vec128;
  wire [31:0]        t_net32;
  reg [31:0]         t_net32_driver;
  
  packed_struct_t    t_packed_struct;

  assign t_net32 = t_net32_driver;

  integer errors = 0;

  // Verification helper macro
  `define assert_eq(val, exp, msg) \
    if ((val) !== (exp)) begin \
      $display("FAIL: %s. Got %p, expected %p", msg, val, exp); \
      errors = errors + 1; \
    end

  initial begin
    $display("=== SystemVerilog Native Force/Release Comprehensive Type Test ===");

    // ----------------------------------------------------
    // TYPE: int (force / release)
    // ----------------------------------------------------
    t_int = 32'h00FF_00FF;
    #1;
    `assert_eq(t_int, 32'h00FF_00FF, "int init")

    force t_int = 32'hCAFE_BABE;
    #1;
    `assert_eq(t_int, 32'hCAFE_BABE, "int force")

    t_int = 32'hBEEF_DEAD;
    #1;
    `assert_eq(t_int, 32'hCAFE_BABE, "int assign blocked during force")

    release t_int;
    #1;
    `assert_eq(t_int, 32'hCAFE_BABE, "int release retain value")

    t_int = 32'hBEEF_DEAD;
    #1;
    `assert_eq(t_int, 32'hBEEF_DEAD, "int post-release assign")

    // ----------------------------------------------------
    // TYPE: byte (assign / deassign)
    // ----------------------------------------------------
    t_byte = 8'h0F;
    #1;
    assign t_byte = 8'hF0;
    #1;
    `assert_eq(t_byte, 8'hF0, "byte procedural assign")

    t_byte = 8'hAA;
    #1;
    `assert_eq(t_byte, 8'hF0, "byte procedural assign block")

    deassign t_byte;
    #1;
    `assert_eq(t_byte, 8'hF0, "byte deassign retain value")

    t_byte = 8'hAA;
    #1;
    `assert_eq(t_byte, 8'hAA, "byte post-deassign assign")

    // ----------------------------------------------------
    // TYPE: real (force / release)
    // ----------------------------------------------------
    t_real = 1.25;
    #1;
    force t_real = 3.1415;
    #1;
    `assert_eq(t_real, 3.1415, "real force")

    t_real = 0.0;
    #1;
    `assert_eq(t_real, 3.1415, "real assign blocked during force")

    release t_real;
    #1;
    `assert_eq(t_real, 3.1415, "real release retain value")

    // ----------------------------------------------------
    // TYPE: shortreal (assign / deassign)
    // ----------------------------------------------------
    t_shortreal = 2.5;
    #1;
    assign t_shortreal = 4.5;
    #1;
    `assert_eq(t_shortreal, 4.5, "shortreal assign")

    deassign t_shortreal;
    #1;
    t_shortreal = 1.0;
    #1;
    `assert_eq(t_shortreal, 1.0, "shortreal post-deassign")

    // ----------------------------------------------------
    // TYPE: bit [127:0] (force / release)
    // ----------------------------------------------------
    t_vec128 = 128'h0;
    #1;
    force t_vec128 = 128'h5A5A5A5A5A5A5A5A_A5A5A5A5A5A5A5A5;
    #1;
    `assert_eq(t_vec128, 128'h5A5A5A5A5A5A5A5A_A5A5A5A5A5A5A5A5, "vec128 force")

    t_vec128 = 128'hFFFF;
    #1;
    `assert_eq(t_vec128, 128'h5A5A5A5A5A5A5A5A_A5A5A5A5A5A5A5A5, "vec128 assign blocked during force")

    release t_vec128;
    #1;
    `assert_eq(t_vec128, 128'h5A5A5A5A5A5A5A5A_A5A5A5A5A5A5A5A5, "vec128 release retain value")

    // ----------------------------------------------------
    // TYPE: wire [31:0] (force / release on net)
    // ----------------------------------------------------
    t_net32_driver = 32'hA5A5_5A5A;
    #1;
    `assert_eq(t_net32, 32'hA5A5_5A5A, "net init driver")

    force t_net32 = 32'hFFFF_FFFF;
    #1;
    `assert_eq(t_net32, 32'hFFFF_FFFF, "net force")

    t_net32_driver = 32'h0000_0000;
    #1;
    `assert_eq(t_net32, 32'hFFFF_FFFF, "net driver change ignored during force")

    release t_net32;
    #1;
    `assert_eq(t_net32, 32'h0000_0000, "net release restores continuous driver")

    // ----------------------------------------------------
    // TYPE: packed struct (force / release / assign / deassign)
    // ----------------------------------------------------
    t_packed_struct.field_a = 16'hAAAA;
    t_packed_struct.field_b = 16'h5555;
    #1;
    `assert_eq(t_packed_struct, {16'hAAAA, 16'h5555}, "packed struct init")

    force t_packed_struct = {16'h1234, 16'h5678};
    #1;
    `assert_eq(t_packed_struct, {16'h1234, 16'h5678}, "packed struct force")

    t_packed_struct = {16'h0000, 16'h0000};
    #1;
    `assert_eq(t_packed_struct, {16'h1234, 16'h5678}, "packed struct assign blocked")

    release t_packed_struct;
    #1;
    `assert_eq(t_packed_struct, {16'h1234, 16'h5678}, "packed struct release retain")

    assign t_packed_struct = {16'hFFFF, 16'h0000};
    #1;
    `assert_eq(t_packed_struct, {16'hFFFF, 16'h0000}, "packed struct procedural assign")

    deassign t_packed_struct;
    #1;

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
