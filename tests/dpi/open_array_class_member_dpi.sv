module open_array_class_member_dpi;
  import "DPI-C" function int oa_inspect_write(
    input string tag,
    output int arr[],
    input int exp_dims,
    input int exp_bytes,
    input int exp_left,
    input int exp_right,
    input int exp_size,
    input int exp_first,
    input int exp_last
  );
  import "DPI-C" function int oa_inspect_write_inout(
    input string tag,
    inout int arr[],
    input int exp_dims,
    input int exp_bytes,
    input int exp_left,
    input int exp_right,
    input int exp_size,
    input int exp_first,
    input int exp_last
  );
  import "DPI-C" function int oa_check_in(
    input string tag,
    input int arr[],
    input int exp_bytes,
    input int exp_left,
    input int exp_right,
    input int exp_first,
    input int exp_last
  );
  import "DPI-C" function int oa_2d_inout(input string tag, inout int arr[][]);

  class Base;
    int base_arr[4];
    function void init_base(int v0, int v3);
      base_arr[0] = v0;
      base_arr[3] = v3;
    endfunction
  endclass

  class C extends Base;
    int m[20];
    int d[];
    int d0[];
    int q[$];
    int q0[$];
    int desc[7:2];
    int asc[2:7];
    int g[2][3];

    function int test_methods(int tag_id);
      int ok = 1;
      m[0] = 42 + tag_id;
      m[19] = 99 + tag_id;
      // output formal zeroes initial values before handing to C (§35.5.6.1)
      ok &= oa_inspect_write("bare_out_m", m, 1, 80, 0, 19, 20, 0, 0);
      if ((m[0] != 100) || (m[19] != 119)) begin
        $display("FAIL bare_out_m_wb m0=%0d m19=%0d", m[0], m[19]);
        ok = 0;
      end
      m[0] = 42 + tag_id;
      m[19] = 99 + tag_id;
      ok &= oa_inspect_write_inout("bare_inout_m", m, 1, 80, 0, 19, 20, 42 + tag_id, 99 + tag_id);
      if ((m[0] != 100) || (m[19] != 119)) begin
        $display("FAIL bare_inout_m_wb m0=%0d m19=%0d", m[0], m[19]);
        ok = 0;
      end

      m[0] = 55;
      m[19] = 66;
      ok &= oa_inspect_write_inout("this_inout_m", this.m, 1, 80, 0, 19, 20, 55, 66);
      if ((this.m[0] != 100) || (this.m[19] != 119)) begin
        $display("FAIL this_inout_m_wb");
        ok = 0;
      end
      ok &= oa_check_in("bare_in_m", m, 80, 0, 19, 100, 119);

      d = new[5];
      d[0] = 7;
      d[4] = 17;
      ok &= oa_inspect_write_inout("bare_d", d, 1, 20, 0, 4, 5, 7, 17);
      if ((d[0] != 100) || (d[4] != 104)) ok = 0;
      ok &= oa_check_in("this_in_d", this.d, 20, 0, 4, 100, 104);

      d0 = new[0];
      ok &= oa_inspect_write("bare_d0", d0, 1, 0, 0, -1, 0, -1, -1);
      ok &= oa_check_in("bare_q0", q0, 0, 0, -1, -1, -1);

      q.delete();
      q.push_back(9);
      q.push_back(8);
      ok &= oa_inspect_write_inout("bare_q", q, 1, 8, 0, 1, 2, 9, 8);
      if ((q[0] != 100) || (q[1] != 101)) ok = 0;

      desc[7] = 77;
      desc[2] = 22;
      ok &= oa_inspect_write_inout("bare_desc", desc, 1, 24, 7, 2, 6, 77, 22);
      if ((desc[2] != 102) || (desc[7] != 107)) begin
        $display("FAIL desc_wb desc2=%0d desc7=%0d", desc[2], desc[7]);
        ok = 0;
      end

      asc[2] = 21;
      asc[7] = 71;
      ok &= oa_inspect_write_inout("bare_asc", asc, 1, 24, 2, 7, 6, 21, 71);
      if ((asc[2] != 102) || (asc[7] != 107)) begin
        $display("FAIL asc_wb asc2=%0d asc7=%0d", asc[2], asc[7]);
        ok = 0;
      end

      for (int i = 0; i < 2; i++)
        for (int j = 0; j < 3; j++)
          g[i][j] = (i + 1) * 10 + j;
      ok &= oa_2d_inout("bare_g2d", g);
      if ((g[0][0] != 200) || (g[1][2] != 212)) ok = 0;

      init_base(31, 34);
      ok &= oa_inspect_write_inout("inherited_bare", base_arr, 1, 16, 0, 3, 4, 31, 34);
      if ((base_arr[0] != 100) || (base_arr[3] != 103)) ok = 0;
      return ok;
    endfunction
  endclass

  int mm[20];
  function automatic int test_local_control();
    int loc[6];
    int ok = 1;
    loc[0] = 5;
    loc[5] = 15;
    ok &= oa_inspect_write_inout("local_ctrl", loc, 1, 24, 0, 5, 6, 5, 15);
    if ((loc[0] != 100) || (loc[5] != 105)) ok = 0;
    return ok;
  endfunction

  initial begin
    C c1 = new();
    C c2 = new();
    int ok = 1;

    ok &= c1.test_methods(0);

    // Module-scope handle-qualified access and two-instance isolation
    c1.m[0] = 11;
    c1.m[19] = 22;
    c2.m[0] = 333;
    c2.m[19] = 444;
    ok &= oa_inspect_write_inout("c1_m_ext", c1.m, 1, 80, 0, 19, 20, 11, 22);
    if ((c1.m[0] != 100) || (c1.m[19] != 119)) ok = 0;
    if ((c2.m[0] != 333) || (c2.m[19] != 444)) begin
      $display("FAIL crosstalk c2.m[0]=%0d c2.m[19]=%0d", c2.m[0], c2.m[19]);
      ok = 0;
    end

    c2.init_base(51, 54);
    ok &= oa_inspect_write_inout("c2_base_ext", c2.base_arr, 1, 16, 0, 3, 4, 51, 54);
    if ((c2.base_arr[0] != 100) || (c2.base_arr[3] != 103)) ok = 0;
    if (c1.base_arr[0] != 100) ok = 0;

    c1.d = new[3];
    c1.d[0] = 10;
    c1.d[2] = 30;
    ok &= oa_check_in("c1_d_ext", c1.d, 12, 0, 2, 10, 30);

    c1.q.delete();
    c1.q.push_back(7);
    ok &= oa_inspect_write_inout("c1_q_ext", c1.q, 1, 4, 0, 0, 1, 7, 7);
    if (c1.q[0] != 100) ok = 0;

    for (int i = 0; i < 2; i++)
      for (int j = 0; j < 3; j++)
        c1.g[i][j] = (i + 1) * 10 + j;
    ok &= oa_2d_inout("c1_g2d_ext", c1.g);
    if (c1.g[1][2] != 212) ok = 0;

    mm[0] = 1;
    mm[19] = 2;
    ok &= oa_inspect_write_inout("mod_ctrl", mm, 1, 80, 0, 19, 20, 1, 2);
    if ((mm[0] != 100) || (mm[19] != 119)) ok = 0;
    ok &= test_local_control();

    if (ok) $display("TEST_PASS");
    else $display("TEST_FAIL");
    $finish;
  end
endmodule
