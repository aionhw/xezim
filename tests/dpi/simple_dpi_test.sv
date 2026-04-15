module simple_dpi_test;
  import "DPI-C" function int add_c(input int a, input int b);

  initial begin
    $display("DPI_RESULT=%0d", add_c(20, 22));
    if (add_c(20, 22) != 42) begin
      $display("TEST_FAIL");
      $finish;
    end
    $display("TEST_PASS");
    $finish;
  end
endmodule
