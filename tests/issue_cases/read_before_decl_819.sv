// §6.5: reads of module data must follow their declarations.
module t;
  logic y1, y2;

  assign y1 = w1;
  logic w1 = 1'b1;

  always_comb y2 = w2;
  logic w2 = 1'b1;

  initial begin
    #1;
    $display("FAIL read before declaration accepted, y1 = %b y2 = %b", y1, y2);
    $finish;
  end
endmodule
