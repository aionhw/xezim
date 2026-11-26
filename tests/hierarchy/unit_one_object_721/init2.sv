int f2_x = f1_c + 1;
module top; m1 u();
  initial begin
    $display("T|init %0d %0d %0d %s %0d", f1_a, f1_b, f1_c, f1_s, f2_x);
    f2_x++; #2 $display("T|end %0d %0d", f1_a, f2_x); $finish;
  end
endmodule
