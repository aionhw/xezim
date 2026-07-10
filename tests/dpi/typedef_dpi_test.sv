// Regression for DPI-C import using a typedef name for a packed vector.
// Before the fix, `hdl_data_t` (typedef for `logic [127:0]`) caused
// `[DPI] unsupported prototype` because dpi_atom_kind() did not resolve
// DataType::TypeReference to the underlying IntegerVector, so the import
// was never bound and calls reported "Undeclared identifier".
//
// This variant additionally exercises the `output` (VecLogicOut) path so
// that a value written on the C side is observed on the SV side.

module typedef_dpi_test;
  typedef logic [127:0] hdl_data_t;

  import "DPI-C" context function int dpi_hdl_deposit(string path, hdl_data_t value);
  import "DPI-C" context function int dpi_hdl_read(string path, output hdl_data_t value);

  // 128-bit all-ones constant so the comparison is width-stable.
  localparam hdl_data_t ALL_ONES = {128{1'b1}};

  initial begin
    hdl_data_t v;
    int rc_dep;
    int rc_rd;

    v = ALL_ONES;
    rc_dep = dpi_hdl_deposit("top.sig", v);

    v = '0;
    rc_rd = dpi_hdl_read("top.sig", v);

    if (rc_dep != 0 && rc_rd != 0 && v === ALL_ONES)
      $display("TEST_PASS");
    else
      $display("TEST_FAIL rc_dep=%0d rc_rd=%0d v=%h", rc_dep, rc_rd, v);
    $finish;
  end

  logic [127:0] sig;
endmodule
