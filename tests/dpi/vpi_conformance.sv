// Driver for tests/dpi/vpi_conformance.c — the IEEE 1800-2017 clause 38
// (VPI) audit's regression coverage. See the C file for what each group
// of checks is guarding against.
module tb;
  // Read targets.
  logic [31:0]  sig32;
  bit   [127:0] wide;
  logic [7:0]   xz;
  logic         one_bit;

  // vpi_get(vpiType) must answer from the DECLARATION, so one signal of
  // each declared type.
  logic [63:0]  w64;
  int           an_int;
  longint       a_long;
  real          a_real;
  wire  [7:0]   a_net;
  logic [7:0]   net_drv;
  assign a_net = net_drv;

  // Write targets.
  logic [7:0]   put_x, put_z, untouched;

  import "DPI-C" context function int vc_names();
  import "DPI-C" context function int vc_get_value();
  import "DPI-C" context function int vc_put_wide();
  import "DPI-C" context function int vc_put_xz();
  import "DPI-C" context function int vc_get_props();
  import "DPI-C" context function int vc_vlog_info();
  import "DPI-C" context function int vc_errors();

  int rc;
  int errors;

  `define check(cond, msg) \
    if (!(cond)) begin errors = errors + 1; $display("FAIL: %s", msg); end

  initial begin
    errors  = 0;
    sig32   = 32'h1234ABCD;
    wide    = 128'h11223344_55667788_99AABBCC_DDEEFF00;
    xz      = 8'b1010_xzxz;
    one_bit = 1'b1;
    w64     = 64'h0;
    an_int  = 0;
    a_long  = 0;
    a_real  = 0.0;
    net_drv = 8'h00;
    put_x   = 8'h00;
    put_z   = 8'h00;
    untouched = 8'hA5;
    #1;

    rc = vc_names();
    rc = vc_get_value();
    rc = vc_get_props();
    rc = vc_vlog_info();

    // vpi_put_value must keep the upper word of a 64-bit signal. The old
    // code read only vec[0].aval for anything <= 64 bits wide.
    rc = vc_put_wide();
    #1;
    `check(w64 === 64'hCCCCDDDD_AAAABBBB, "64-bit vector put lost its upper word")

    // ...and must carry bval through, so X and Z survive a deposit.
    rc = vc_put_xz();
    #1;
    `check(put_x === 8'bxxxx1111, "aval=1,bval=1 must deposit X")
    `check(put_z === 8'bzzzz1111, "aval=0,bval=1 must deposit Z")
    // An undecodable format writes nothing rather than writing the signal
    // back to itself and looking like a successful no-op.
    `check(untouched === 8'hA5, "an unsupported put format must not write")

    errors = errors + vc_errors();
    if (errors == 0) $display("RESULT: PASSED");
    else             $display("RESULT: FAILED (%0d errors)", errors);
  end
endmodule
