// The SV side of mixed_array_of_arrays.vhd: arr2 is 3 x 2 bytes, element
// (i)(j) at bits 47 - 16 * i - 8 * j downward.
module mixed_array_of_arrays;
  logic [47:0] d;
  wire [47:0] q;
  wire [7:0] e0, e3, e5;

  aoa_io dut (.d(d), .q(q), .e0(e0), .e3(e3), .e5(e5));

  initial begin
    d = 48'h12345678_9ABC;
    #1;
    $display("q=%h e0=%h e3=%h e5=%h", q, e0, e3, e5);
    d = 48'hC0FFEE_1234_56;
    #1;
    $display("q=%h e0=%h e3=%h e5=%h", q, e0, e3, e5);
    $finish;
  end
endmodule
