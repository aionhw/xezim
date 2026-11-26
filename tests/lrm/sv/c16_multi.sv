// top: c16_multi
module c16_multi;
  // The trace records nonvacuous matches and failures.
  initial $assertvacuousoff(0);
  bit c1, c2; bit a, b, req, gnt;
  always #5 c1 = ~c1;
  always #7 c2 = ~c2;
  initial begin
    a = 0; b = 0;
    #12 a = 1; #10 a = 0;
    #3 b = 1; #20 b = 0;
    #40 a = 1; #10 a = 0;
    #60 $finish;
  end
  mc1: assert property (@(posedge c1) a |=> @(posedge c2) b) $display("T|16.13|mc1 P t=%0t", $time); else $display("T|16.13|mc1 F t=%0t", $time);
  mc2: assert property (@(posedge c1) a ##1 @(posedge c2) b) $display("T|16.13|mc2 P t=%0t", $time); else $display("T|16.13|mc2 F t=%0t", $time);
  // expect
  initial begin
    #2 req = 1; #10;
    expect (@(posedge c1) req ##2 gnt) $display("T|16.17|expect P t=%0t", $time); else $display("T|16.17|expect F t=%0t", $time);
    $display("T|16.17|after expect t=%0t", $time);
  end
  initial begin #30 gnt = 1; end
  // assume / restrict / cover sequence
  as1: assume property (@(posedge c1) !(a && b)) else $display("T|16.14.2|assume F t=%0t", $time);
  cs: cover sequence (@(posedge c1) a ##1 !a) $display("T|16.14.3|cover seq t=%0t", $time);
endmodule
