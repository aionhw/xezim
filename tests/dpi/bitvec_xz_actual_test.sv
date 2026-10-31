// A logic actual with x / z bits bound to a two-state `bit [N]` DPI formal
// converts as by assignment (IEEE 1800-2017 §35.6.1, §6.11.2): the unknown
// bits reach C as 0.
module bitvec_xz_actual;
  import "DPI-C" function void bxz_in(input bit [31:0] v);
  import "DPI-C" function void bxz_in_wide(input bit [69:0] v);
  import "DPI-C" function void bxz_inout(inout bit [7:0] v);
  import "DPI-C" function int bxz_seen(int i);
  int f = 0;
  `define CK(e, m) if (!(e)) begin f++; $display("FAIL %s", m); end
  logic [31:0] l32;
  logic [69:0] l70;
  logic [7:0] l8;

  initial begin
    l32 = 32'h8000_00x1;
    l32[7] = 1'bz;
    bxz_in(l32);
    `CK(bxz_seen(0) == 32'h8000_0001, "bit [31:0] in drops x and z")
    bxz_in(32'bx);
    `CK(bxz_seen(0) == 0, "all-x literal")
    l70 = {6'b10_xz01, 32'hx000_0003, 32'h0000_00zf};
    bxz_in_wide(l70);
    `CK(bxz_seen(0) == 32'h0000_000f, "bit [69:0] in word 0")
    `CK(bxz_seen(1) == 32'h0000_0003, "bit [69:0] in word 1")
    `CK(bxz_seen(2) == 32'h21, "bit [69:0] in word 2")
    l8 = 8'bx1z0_0001;
    bxz_inout(l8);
    `CK(bxz_seen(0) == 8'b0100_0001, "bit [7:0] inout seeds known bits only")
    `CK(l8 === 8'h5, "bit [7:0] inout writes back")
    if (f == 0) $display("TEST_PASS"); else $display("TEST_FAIL count=%0d", f);
    $finish;
  end
endmodule
