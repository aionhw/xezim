module shortreal_string_dpi_test;
  import "DPI-C" function shortreal add_sr(input shortreal a, input shortreal b);
  import "DPI-C" function void scale_sr(inout shortreal x, input shortreal s);
  import "DPI-C" function void set_msg(output string s);
  import "DPI-C" function void append_msg(inout string s);

  shortreal x;
  shortreal y;
  string s;

  initial begin
    x = 1.25;
    y = add_sr(1.5, 2.0);
    scale_sr(x, 4.0);

    set_msg(s);
    append_msg(s);

    $display("DPI_SR_STR x=%0f y=%0f s=%s", x, y, s);
    if ((x > 4.99) && (x < 5.01) &&
        (y > 3.49) && (y < 3.51) &&
        (s == "hello_out_tail")) begin
      $display("TEST_PASS");
    end else begin
      $display("TEST_FAIL");
    end
    $finish;
  end
endmodule
