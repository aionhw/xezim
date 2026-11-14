// IEEE 1800-2023 §3.12.1 / §16.8 / §16.12: unit declarations retain lexical scope.
sequence good(logic x); x; endsequence
property rule(logic x); good(x); endproperty
property unused_rule(logic x); !x; endproperty
module child(input logic clk);
  int passed=0, failed=0;
  a: assert property (@(posedge clk) rule(1'b1)) passed++; else failed++;
endmodule
module shadow(input logic clk);
  int passed=0, failed=0;
  property rule(logic x); !x; endproperty
  a: assert property (@(posedge clk) rule(1'b0)) passed++; else failed++;
endmodule
module unrelated; endmodule
module top;
  logic clk=0;
  always #5 clk=~clk;
  child a(clk), b(clk);
  shadow c(clk);
  unrelated d();
  initial begin
    #31;
    if(a.passed!=3 || b.passed!=3 || c.passed!=3 || a.failed || b.failed || c.failed)
      $fatal(1,"assertion scope: %0d/%0d %0d/%0d %0d/%0d",a.passed,a.failed,b.passed,b.failed,c.passed,c.failed);
    $display("PASS unit assertions");
    $finish;
  end
endmodule
