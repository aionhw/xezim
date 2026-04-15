module open_array_dpi_test;
  import "DPI-C" function int sum4(input int a[]);
  import "DPI-C" function void fill4(output int a[]);
  import "DPI-C" function void bump4(inout int a[]);

  int a[0:3];
  int s;

  initial begin
    a[0] = 1; a[1] = 2; a[2] = 3; a[3] = 4;
    s = sum4(a);
    bump4(a);
    if (!((s == 10) && (a[0] == 2) && (a[1] == 3) && (a[2] == 4) && (a[3] == 5))) begin
      $display("TEST_FAIL");
      $finish;
    end
    fill4(a);
    $display("DPI_OA s=%0d a=%0d,%0d,%0d,%0d", s, a[0], a[1], a[2], a[3]);
    if ((a[0] == 10) && (a[1] == 20) && (a[2] == 30) && (a[3] == 40)) begin
      $display("TEST_PASS");
    end else begin
      $display("TEST_FAIL");
    end
    $finish;
  end
endmodule
